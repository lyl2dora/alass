#[cfg(not(any(feature = "ffmpeg-binary", feature = "ffmpeg-library")))]
compile_error!("alass-cli needs an audio backend: enable `ffmpeg-binary` (the default) or `ffmpeg-library`");

// The two backends are alternatives, and `ffmpeg-library` wins when both are on. Features are
// additive in cargo, so refusing the combination would break `--all-features` for every tool
// that uses it.
#[cfg(feature = "ffmpeg-library")]
mod ffmpeg_library;

#[cfg(feature = "ffmpeg-library")]
pub use ffmpeg_library::{DecoderError, VideoDecoderFFmpegLibrary as VideoDecoder};

#[cfg(all(feature = "ffmpeg-binary", not(feature = "ffmpeg-library")))]
mod ffmpeg_binary;

#[cfg(all(feature = "ffmpeg-binary", not(feature = "ffmpeg-library")))]
pub use ffmpeg_binary::{DecoderError, VideoDecoderFFmpegBinary as VideoDecoder};

/// Receives the audio samples a [`VideoDecoder`] reads out of a video file.
pub trait AudioReceiver {
    type Output;
    type Error: std::error::Error + Send + Sync + 'static;

    /// Samples are in 8000Hz mono/single-channel format.
    fn push_samples(&mut self, samples: &[i16]) -> Result<(), Self::Error>;

    fn finish(self) -> Result<Self::Output, Self::Error>;
}

/// Hands the samples on in fixed-size chunks, because the voice activity detector
/// only accepts exactly 10ms of audio at a time.
#[derive(Debug)]
pub struct ChunkedAudioReceiver<R: AudioReceiver> {
    buffer: Vec<i16>,
    filled: usize,
    next: R,
}

impl<R: AudioReceiver> ChunkedAudioReceiver<R> {
    pub fn new(size: usize, next: R) -> ChunkedAudioReceiver<R> {
        ChunkedAudioReceiver {
            buffer: vec![0; size],
            filled: 0,
            next,
        }
    }
}

impl<R: AudioReceiver> AudioReceiver for ChunkedAudioReceiver<R> {
    type Output = R::Output;
    type Error = R::Error;

    fn push_samples(&mut self, mut samples: &[i16]) -> Result<(), R::Error> {
        assert!(self.buffer.len() > self.filled);

        while !samples.is_empty() {
            let sample_count = std::cmp::min(self.buffer.len() - self.filled, samples.len());
            self.buffer[self.filled..self.filled + sample_count].copy_from_slice(&samples[..sample_count]);

            samples = &samples[sample_count..];
            self.filled += sample_count;

            if self.filled == self.buffer.len() {
                self.next.push_samples(self.buffer.as_slice())?;
                self.filled = 0;
            }
        }

        Ok(())
    }

    fn finish(self) -> Result<R::Output, R::Error> {
        self.next.finish()
    }
}

/// Use this trait if you want more detailed information about the progress of operations.
pub trait ProgressHandler {
    /// Will be called one time before `inc()` is called. `steps` is the
    /// number of times `inc()` will be called.
    ///
    /// The number of steps is around the number of lines in the "incorrect" subtitle.
    /// Be aware that this number can be zero!
    fn init(&mut self, steps: i64) {
        let _ = steps;
    }

    /// We made (small) progress!
    fn inc(&mut self) {}

    /// Will be called after the last `inc()`, when `inc()` was called `steps` times.
    fn finish(&mut self) {}
}
