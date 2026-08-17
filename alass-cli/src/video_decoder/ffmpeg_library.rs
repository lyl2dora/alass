//! Audio extraction through the FFmpeg *libraries* (`ffmpeg-next`).
//!
//! This is a drop-in twin of `ffmpeg_binary::VideoDecoderFFmpegBinary`: it produces the very same
//! 8kHz mono `i16` sample stream, only without spawning `ffprobe`/`ffmpeg` subprocesses.
//!
//! Nothing in here writes to stdout - the CLI reserves stdout for its alignment report - and
//! FFmpeg's own logging is silenced so it cannot scribble over the progress bar.

use std::path::{Path, PathBuf};
use std::sync::Once;

use ffmpeg_next as ffmpeg;

use ffmpeg::format::sample::Type as SampleType;
use ffmpeg::media::Type as MediaType;
use ffmpeg::util::error::EAGAIN;
use ffmpeg::util::format::Sample as SampleFormat;
use ffmpeg::{ChannelLayout, Packet, frame};

/// The voice activity detector expects 8kHz mono audio.
const TARGET_SAMPLE_RATE: u32 = 8000;

/// One `progress_handler.inc()` per this many samples (same convention as the binary decoder).
const PROGRESS_PRESCALER: i64 = 200;

/// Every error the library decoder can produce.
///
/// The variants keep the underlying [`ffmpeg::Error`] as a `source()` so the CLI can print it as a
/// `caused by:` line instead of mashing it into one string.
#[derive(Debug, thiserror::Error)]
pub enum DecoderError {
    #[error("failed to initialize the FFmpeg libraries")]
    InitializationFailed(#[source] ffmpeg::Error),

    #[error("failed to open media file '{path}'")]
    OpeningFileFailed {
        path: PathBuf,
        #[source]
        source: ffmpeg::Error,
    },

    #[error("no audio stream in file '{path}'")]
    NoAudioStream { path: PathBuf },

    #[error("no audio duration information found for file '{path}'")]
    NoDurationInformation { path: PathBuf },

    #[error("failed to open a decoder for stream {stream_index} of file '{path}'")]
    OpeningDecoderFailed {
        path: PathBuf,
        stream_index: usize,
        #[source]
        source: ffmpeg::Error,
    },

    #[error("failed to read a packet from file '{path}'")]
    ReadingPacketFailed {
        path: PathBuf,
        #[source]
        source: ffmpeg::Error,
    },

    #[error("failed to decode audio stream {stream_index} of file '{path}'")]
    FailedToDecodeAudio {
        path: PathBuf,
        stream_index: usize,
        #[source]
        source: ffmpeg::Error,
    },

    #[error("failed to resample audio stream {stream_index} of file '{path}' to {TARGET_SAMPLE_RATE}Hz mono")]
    ResamplingFailed {
        path: PathBuf,
        stream_index: usize,
        #[source]
        source: ffmpeg::Error,
    },

    #[error("processing audio segment failed")]
    AudioSegmentProcessingFailed(#[source] Box<dyn std::error::Error + Send + Sync + 'static>),
}

/// Initializes FFmpeg once per process and keeps its logging off our terminal.
fn init_ffmpeg() -> Result<(), DecoderError> {
    static SILENCE_LOG: Once = Once::new();

    ffmpeg::init().map_err(DecoderError::InitializationFailed)?;

    SILENCE_LOG.call_once(|| {
        // The CLI keeps stdout for its alignment report and draws a progress bar on stderr;
        // FFmpeg's own diagnostics would scribble over both.
        ffmpeg::util::log::set_level(ffmpeg::util::log::Level::Quiet);
    });

    Ok(())
}

#[derive(Debug)]
pub struct VideoDecoderFFmpegLibrary {}

impl VideoDecoderFFmpegLibrary {
    /// Samples are pushed in 8kHz mono/single-channel format.
    pub fn decode<T>(
        file_path: impl AsRef<Path>,
        audio_index: Option<usize>,
        mut receiver: impl super::AudioReceiver<Output = T>,
        mut progress_handler: impl super::ProgressHandler,
    ) -> Result<T, DecoderError> {
        let path: PathBuf = file_path.as_ref().into();

        init_ffmpeg()?;

        let mut input = ffmpeg::format::input(&path).map_err(|source| DecoderError::OpeningFileFailed {
            path: path.clone(),
            source,
        })?;

        // --- pick the audio stream -------------------------------------------------------------
        //
        // Same rule as the binary decoder: an explicit `--index` selects by *stream* index,
        // otherwise the audio stream with the fewest channels wins (it resamples fastest).
        let mut best: Option<AudioStreamChoice> = None;

        for stream in input.streams() {
            if stream.parameters().medium() != MediaType::Audio {
                continue;
            }
            if let Some(wanted) = audio_index {
                if stream.index() != wanted {
                    continue;
                }
            }

            let stream_index = stream.index();

            let decoder = ffmpeg::codec::context::Context::from_parameters(stream.parameters())
                .and_then(|context| context.decoder().audio());

            let decoder = match decoder {
                Ok(decoder) => decoder,
                Err(source) => {
                    // An explicitly requested stream that cannot be opened is a hard error;
                    // otherwise just skip it, like `ffprobe` skips streams without a channel count.
                    if audio_index.is_some() {
                        return Err(DecoderError::OpeningDecoderFailed {
                            path,
                            stream_index,
                            source,
                        });
                    }
                    continue;
                }
            };

            let channels = decoder.channels();
            if channels == 0 {
                // mirrors the binary decoder's `s.channels.is_some()` filter
                continue;
            }

            let duration = stream_duration_in_seconds(&stream);

            if best.as_ref().is_none_or(|current| channels < current.channels) {
                best = Some(AudioStreamChoice {
                    stream_index,
                    channels,
                    duration,
                    decoder,
                });
            }
        }

        let AudioStreamChoice {
            stream_index,
            channels: _,
            duration,
            mut decoder,
        } = match best {
            Some(choice) => choice,
            None => return Err(DecoderError::NoAudioStream { path }),
        };

        // `.mkv` containers do not store a duration in the stream, only the container does.
        let duration = duration
            .or_else(|| container_duration_in_seconds(&input))
            .ok_or_else(|| DecoderError::NoDurationInformation { path: path.clone() })?;

        progress_handler.init((duration * f64::from(TARGET_SAMPLE_RATE)) as i64 / PROGRESS_PRESCALER);

        // --- decode / resample -----------------------------------------------------------------
        let mut state = DecodeState {
            path: &path,
            stream_index,
            resampler: None,
            input_definition: None,
            out: OutputBuffer::new(),
            samples_since_inc: 0,
        };

        let mut packet = Packet::empty();
        let mut decoded = frame::Audio::empty();

        loop {
            match packet.read(&mut input) {
                Ok(()) => {}
                Err(ffmpeg::Error::Eof) => break,
                Err(source) => {
                    return Err(DecoderError::ReadingPacketFailed {
                        path: path.clone(),
                        source,
                    });
                }
            }

            if packet.stream() != stream_index {
                continue;
            }

            decoder
                .send_packet(&packet)
                .map_err(|source| DecoderError::FailedToDecodeAudio {
                    path: path.clone(),
                    stream_index,
                    source,
                })?;

            state.drain_decoder(&mut decoder, &mut decoded, &mut receiver, &mut progress_handler)?;
        }

        // flush the decoder ...
        decoder.send_eof().map_err(|source| DecoderError::FailedToDecodeAudio {
            path: path.clone(),
            stream_index,
            source,
        })?;
        state.drain_decoder(&mut decoder, &mut decoded, &mut receiver, &mut progress_handler)?;

        // ... and then the resampler, which keeps a few samples of its own.
        state.flush_resampler(&mut receiver, &mut progress_handler)?;

        progress_handler.finish();

        receiver
            .finish()
            .map_err(|e| DecoderError::AudioSegmentProcessingFailed(Box::new(e)))
    }
}

struct AudioStreamChoice {
    stream_index: usize,
    channels: u16,
    duration: Option<f64>,
    decoder: ffmpeg::decoder::Audio,
}

/// Duration of a stream in seconds, or `None` if the container does not know it.
fn stream_duration_in_seconds(stream: &ffmpeg::Stream<'_>) -> Option<f64> {
    let duration = stream.duration();
    if duration == ffmpeg::ffi::AV_NOPTS_VALUE || duration < 0 {
        return None;
    }
    Some(duration as f64 * f64::from(stream.time_base()))
}

/// Duration of the whole container in seconds, or `None` if it does not know it.
fn container_duration_in_seconds(input: &ffmpeg::format::context::Input) -> Option<f64> {
    let duration = input.duration();
    if duration == ffmpeg::ffi::AV_NOPTS_VALUE || duration < 0 {
        return None;
    }
    Some(duration as f64 / f64::from(ffmpeg::ffi::AV_TIME_BASE))
}

/// Mirrors what `ffmpeg`'s `aresample` filter does in `config_output()`.
///
/// (E-)AC-3, DTS and TrueHD carry the mastering engineer's downmix levels in the bitstream; the
/// decoder exports them as `AV_FRAME_DATA_DOWNMIX_INFO`. The `ffmpeg` binary feeds them to the
/// resampler, so a 5.1 stream mixed down to mono comes out several percent quieter than the
/// resampler's own defaults would make it - enough to move voice-activity boundaries. Since we
/// always mix down to mono (never stereo), `aresample` uses the plain, non-Lt/Rt levels and leaves
/// `matrix_encoding` at "none".
fn downmix_options(frame: &frame::Audio) -> ffmpeg::Dictionary<'static> {
    /// Field offsets in `AVDownmixInfo` (`libavutil/downmix_info.h`); the struct is 48 bytes:
    /// an enum, then six `double`s at 8-byte offsets.
    const CENTER_MIX_LEVEL: usize = 8;
    const SURROUND_MIX_LEVEL: usize = 24;
    const LFE_MIX_LEVEL: usize = 40;
    const SIZE: usize = 48;

    let mut options = ffmpeg::Dictionary::new();

    let Some(side_data) = frame.side_data(frame::side_data::Type::DownMixInfo) else {
        return options;
    };
    let data = side_data.data();
    if data.len() < SIZE {
        return options;
    }

    let level = |offset: usize| {
        let bytes: [u8; 8] = data[offset..offset + 8].try_into().expect("slice is 8 bytes long");
        f64::from_ne_bytes(bytes)
    };

    options.set("clev", &level(CENTER_MIX_LEVEL).to_string());
    options.set("slev", &level(SURROUND_MIX_LEVEL).to_string());
    options.set("lfe_mix_level", &level(LFE_MIX_LEVEL).to_string());

    options
}

/// A reusable output frame for the resampler.
///
/// `swr_convert_frame` uses the frame's `nb_samples` as the output *capacity* and then overwrites
/// it with the number of samples it actually produced, so the capacity has to be restored before
/// every conversion.
struct OutputBuffer {
    frame: frame::Audio,
    capacity: usize,
}

impl OutputBuffer {
    fn new() -> Self {
        OutputBuffer {
            frame: frame::Audio::empty(),
            capacity: 0,
        }
    }

    /// Returns a frame with room for at least `wanted` samples and `nb_samples == capacity`.
    fn prepare(&mut self, wanted: usize) -> &mut frame::Audio {
        if wanted > self.capacity {
            let capacity = wanted.next_power_of_two().max(1024);
            self.frame = frame::Audio::new(SampleFormat::I16(SampleType::Packed), capacity, ChannelLayout::MONO);
            self.capacity = capacity;
        } else {
            self.frame.set_samples(self.capacity);
        }
        &mut self.frame
    }
}

struct DecodeState<'a> {
    path: &'a Path,
    stream_index: usize,
    resampler: Option<ffmpeg::software::resampling::Context>,
    /// The frame layout the current resampler was built for. `swr_convert_frame` rejects a frame
    /// that does not match it, so a mid-stream format change means: flush, then rebuild.
    input_definition: Option<(SampleFormat, ChannelLayout, u32)>,
    out: OutputBuffer,
    samples_since_inc: i64,
}

impl DecodeState<'_> {
    fn decode_error(&self, source: ffmpeg::Error) -> DecoderError {
        DecoderError::FailedToDecodeAudio {
            path: self.path.into(),
            stream_index: self.stream_index,
            source,
        }
    }

    /// Pull every frame the decoder currently has, resample it and hand it to the receiver.
    fn drain_decoder<T>(
        &mut self,
        decoder: &mut ffmpeg::decoder::Audio,
        decoded: &mut frame::Audio,
        receiver: &mut impl super::AudioReceiver<Output = T>,
        progress_handler: &mut impl super::ProgressHandler,
    ) -> Result<(), DecoderError> {
        loop {
            match decoder.receive_frame(decoded) {
                Ok(()) => {}
                // "send me more data" and "that was everything" both just end this round
                Err(ffmpeg::Error::Eof) => return Ok(()),
                Err(ffmpeg::Error::Other { errno }) if errno == EAGAIN => return Ok(()),
                Err(source) => return Err(self.decode_error(source)),
            }

            if decoded.samples() == 0 {
                continue;
            }

            self.resample_frame(decoded, receiver, progress_handler)?;
        }
    }

    fn resample_frame<T>(
        &mut self,
        decoded: &mut frame::Audio,
        receiver: &mut impl super::AudioReceiver<Output = T>,
        progress_handler: &mut impl super::ProgressHandler,
    ) -> Result<(), DecoderError> {
        // Raw PCM in a Matroska container arrives with an *unspecified* channel layout. `swr_init`
        // normalizes such a layout to the native default, and `swr_convert_frame` then rejects
        // every frame as "input changed" because the frame still carries the unspecified one.
        // Give the frame the same normalized layout up front.
        if decoded.channel_layout().is_empty() {
            let channels = decoded.channel_layout().channels().max(1);
            decoded.set_channel_layout(ChannelLayout::default(channels));
        }

        let in_rate = decoded.rate().max(1);
        let in_format = decoded.format();
        let in_layout = decoded.channel_layout();
        let definition = (in_format, in_layout, in_rate);

        // A mid-stream format change needs a new resampler - but the old one is still holding
        // samples, so drain it first.
        if self.input_definition.is_some_and(|current| current != definition) {
            self.flush_resampler(receiver, progress_handler)?;
            self.resampler = None;
            self.input_definition = None;
        }

        if self.resampler.is_none() {
            let resampler = ffmpeg::software::resampling::Context::get_with(
                in_format,
                in_layout,
                in_rate,
                SampleFormat::I16(SampleType::Packed),
                ChannelLayout::MONO,
                TARGET_SAMPLE_RATE,
                downmix_options(decoded),
            )
            .map_err(|source| DecoderError::ResamplingFailed {
                path: self.path.into(),
                stream_index: self.stream_index,
                source,
            })?;

            self.resampler = Some(resampler);
            self.input_definition = Some(definition);
        }

        let resampler = self.resampler.as_mut().expect("resampler was just created");

        // Everything swr is still holding onto, plus this frame, rescaled to the output rate.
        let buffered = resampler.delay().map(|delay| delay.output).unwrap_or(0).max(0) as usize;
        let wanted = buffered
            + (decoded.samples() as u64 * u64::from(TARGET_SAMPLE_RATE)).div_ceil(u64::from(in_rate)) as usize
            + 32;

        let out = self.out.prepare(wanted);
        resampler
            .run(decoded, out)
            .map_err(|source| DecoderError::ResamplingFailed {
                path: self.path.into(),
                stream_index: self.stream_index,
                source,
            })?;

        let produced = out.samples();
        if produced > 0 {
            let samples = &out.plane::<i16>(0)[..produced];
            receiver
                .push_samples(samples)
                .map_err(|e| DecoderError::AudioSegmentProcessingFailed(Box::new(e)))?;
            advance_progress(&mut self.samples_since_inc, produced, progress_handler);
        }

        Ok(())
    }

    /// Drain the samples swr is still buffering internally.
    fn flush_resampler<T>(
        &mut self,
        receiver: &mut impl super::AudioReceiver<Output = T>,
        progress_handler: &mut impl super::ProgressHandler,
    ) -> Result<(), DecoderError> {
        let Some(resampler) = self.resampler.as_mut() else {
            return Ok(());
        };

        while let Some(delay) = resampler.delay() {
            let wanted = (delay.output.max(0) as usize) + 32;
            let out = self.out.prepare(wanted);

            resampler.flush(out).map_err(|source| DecoderError::ResamplingFailed {
                path: self.path.into(),
                stream_index: self.stream_index,
                source,
            })?;

            let produced = out.samples();
            if produced == 0 {
                break;
            }

            let samples = &out.plane::<i16>(0)[..produced];
            receiver
                .push_samples(samples)
                .map_err(|e| DecoderError::AudioSegmentProcessingFailed(Box::new(e)))?;
            advance_progress(&mut self.samples_since_inc, produced, progress_handler);
        }

        Ok(())
    }
}

fn advance_progress(counter: &mut i64, produced: usize, progress_handler: &mut impl super::ProgressHandler) {
    *counter += produced as i64;
    while *counter >= PROGRESS_PRESCALER {
        *counter -= PROGRESS_PRESCALER;
        progress_handler.inc();
    }
}
