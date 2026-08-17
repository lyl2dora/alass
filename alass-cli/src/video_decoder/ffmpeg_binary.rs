//! Reads the audio out of a video file by driving the `ffmpeg`/`ffprobe` executables.
//!
//! `ffprobe` reports the streams and the duration, then `ffmpeg` is asked for the
//! smallest audio stream as raw 8kHz mono 16-bit samples on stdout.

use serde::{Deserialize, Deserializer};
use std::ffi::OsString;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdout, Command, Output, Stdio};
use std::str::from_utf8;
use thiserror::Error;

#[derive(Debug, PartialEq, Eq)]
pub enum CodecType {
    Audio,
    Video,
    Subtitle,
    Other(String),
}

impl<'de> Deserialize<'de> for CodecType {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        Ok(match s.as_str() {
            "audio" => CodecType::Audio,
            "video" => CodecType::Video,
            "subtitle" => CodecType::Subtitle,
            _ => CodecType::Other(s),
        })
    }
}

#[derive(Debug, Deserialize)]
struct Stream {
    index: usize,
    channels: Option<usize>,
    /// `.mkv` does not store the duration in the streams; we have to use `format -> duration` instead
    duration: Option<String>,
    codec_type: CodecType,
}

#[derive(Debug, Deserialize)]
struct Format {
    duration: Option<String>,
}

/// Metadata associated with a video.
#[derive(Debug, Deserialize)]
struct Metadata {
    streams: Vec<Stream>,
    format: Option<Format>,
}

/// Everything that can go wrong while getting audio samples out of a video file.
#[derive(Debug, Error)]
pub enum DecoderError {
    #[error("failed to decode video stream info")]
    FailedToDecodeVideoStreamInfo(#[source] std::str::Utf8Error),

    #[error(
        "failed to extract metadata from '{}' using command '{}'",
        file_path.display(),
        format_cmd(cmd_path, args)
    )]
    ExtractingMetadataFailed {
        cmd_path: PathBuf,
        file_path: PathBuf,
        args: Vec<OsString>,
        #[source]
        source: Box<DecoderError>,
    },

    #[error("no audio stream in file '{}'", path.display())]
    NoAudioStream { path: PathBuf },

    #[error(
        "failed to extract audio from '{}' with '{}'",
        file_path.display(),
        format_cmd(cmd_path, args)
    )]
    FailedExtractingAudio {
        file_path: PathBuf,
        cmd_path: PathBuf,
        args: Vec<OsString>,
        #[source]
        source: Box<DecoderError>,
    },

    #[error("failed to spawn subprocess '{}'", format_cmd(path, args))]
    FailedSpawningSubprocess {
        path: PathBuf,
        args: Vec<OsString>,
        #[source]
        source: std::io::Error,
    },

    #[error("failed to check status of subprocess '{}'", cmd_path.display())]
    WaitingForProcessFailed {
        cmd_path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error(
        "process '{}' returned error code '{}'",
        cmd_path.display(),
        code.map_or_else(|| String::from("interrupted?"), |code| code.to_string())
    )]
    ProcessErrorCode {
        cmd_path: PathBuf,
        code: Option<i32>,
        /// the process' stderr, when it said anything at all
        #[source]
        source: Option<Box<DecoderError>>,
    },

    #[error("stderr: {msg}")]
    ProcessErrorMessage { msg: String },

    #[error("failed to deserialize metadata of file '{}'", path.display())]
    DeserializingMetadataFailed {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },

    #[error("error while reading stdout")]
    ReadError(#[source] std::io::Error),

    #[error("failed to parse duration string '{s}' from metadata")]
    FailedToParseDuration {
        s: String,
        #[source]
        source: std::num::ParseFloatError,
    },

    /// The cause is an [`AudioReceiver::Error`](super::AudioReceiver::Error), which in
    /// practice has `DecoderError` somewhere in its own source chain - the box breaks
    /// that type cycle.
    #[error("processing audio segment failed")]
    AudioSegmentProcessingFailed(#[source] Box<dyn std::error::Error + Send + Sync + 'static>),

    #[error("no audio duration information found")]
    NoDurationInformation,
}

fn format_cmd(cmd_path: &Path, args: &[OsString]) -> String {
    let args = args
        .iter()
        .map(|arg| arg.to_string_lossy())
        .collect::<Vec<_>>()
        .join(" ");
    format!("{} {}", cmd_path.display(), args)
}

/// Extracts the audio of a video file with the `ffmpeg` command line tool.
#[derive(Debug)]
pub struct VideoDecoderFFmpegBinary {}

/// How many samples one step of the progress bar stands for.
const PROGRESS_PRESCALER: i64 = 200;

/// The samples are read from the pipe in blocks of this size.
const READ_BUFFER_SIZE: usize = 1024 * 1024;

impl VideoDecoderFFmpegBinary {
    /// Samples are pushed in 8kHz mono/single-channel format.
    pub fn decode<T>(
        file_path: impl AsRef<Path>,
        audio_index: Option<usize>,
        receiver: impl super::AudioReceiver<Output = T>,
        mut progress_handler: impl super::ProgressHandler,
    ) -> Result<T, DecoderError> {
        let file_path = file_path.as_ref();

        let ffprobe_args = vec![
            OsString::from("-v"),
            OsString::from("error"),
            OsString::from("-show_entries"),
            OsString::from("format=duration:stream=index,channels,duration,codec_type"),
            OsString::from("-of"),
            OsString::from("json"),
            OsString::from(file_path),
        ];

        let ffprobe_path: PathBuf = std::env::var_os("ALASS_FFPROBE_PATH")
            .unwrap_or_else(|| OsString::from("ffprobe"))
            .into();

        let metadata: Metadata = Self::get_metadata(file_path, &ffprobe_path, &ffprobe_args).map_err(|source| {
            DecoderError::ExtractingMetadataFailed {
                cmd_path: ffprobe_path,
                file_path: file_path.to_path_buf(),
                args: ffprobe_args,
                source: Box::new(source),
            }
        })?;

        let mut audio_streams = metadata
            .streams
            .into_iter()
            .filter(|stream| stream.codec_type == CodecType::Audio && stream.channels.is_some());

        let best_stream = match audio_index {
            None => audio_streams.min_by_key(|stream| stream.channels.unwrap()),
            Some(audio_index) => audio_streams.find(|stream| stream.index == audio_index),
        };
        let Some(best_stream) = best_stream else {
            return Err(DecoderError::NoAudioStream {
                path: file_path.to_path_buf(),
            });
        };

        let ffmpeg_path: PathBuf = std::env::var_os("ALASS_FFMPEG_PATH")
            .unwrap_or_else(|| OsString::from("ffmpeg"))
            .into();

        let ffmpeg_args: Vec<OsString> = vec![
            // only print errors
            OsString::from("-v"),
            OsString::from("error"),
            // "yes" -> disables user interaction
            OsString::from("-y"),
            // input file
            OsString::from("-i"),
            file_path.into(),
            // select stream
            OsString::from("-map"),
            format!("0:{}", best_stream.index).into(),
            // audio codec: 16-bit signed little endian
            OsString::from("-acodec"),
            OsString::from("pcm_s16le"),
            // resample to 8khz
            OsString::from("-ar"),
            OsString::from("8000"),
            // resample to single channel
            OsString::from("-ac"),
            OsString::from("1"),
            // output 16-bit signed little endian stream directly (no wav, etc.)
            OsString::from("-f"),
            OsString::from("s16le"),
            // output to stdout pipe
            OsString::from("-"),
        ];

        // `.mkv` containers do not store duration info in streams, only the format information does contain it
        let duration_str = best_stream
            .duration
            .or_else(|| metadata.format.and_then(|format| format.duration))
            .ok_or(DecoderError::NoDurationInformation)?;

        let duration = duration_str
            .parse::<f64>()
            .map_err(|source| DecoderError::FailedToParseDuration {
                s: duration_str,
                source,
            })?;

        progress_handler.init((duration * 8000.0) as i64 / PROGRESS_PRESCALER);

        Self::extract_audio_stream(receiver, progress_handler, &ffmpeg_path, &ffmpeg_args).map_err(|source| {
            DecoderError::FailedExtractingAudio {
                file_path: file_path.to_path_buf(),
                cmd_path: ffmpeg_path,
                args: ffmpeg_args,
                source: Box::new(source),
            }
        })
    }

    fn extract_audio_stream<T>(
        mut receiver: impl super::AudioReceiver<Output = T>,
        mut progress_handler: impl super::ProgressHandler,
        ffmpeg_path: &Path,
        args: &[OsString],
    ) -> Result<T, DecoderError> {
        let mut ffmpeg_process: Child = Command::new(ffmpeg_path)
            .args(args)
            .stdin(Stdio::null())
            .stderr(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .map_err(|source| DecoderError::FailedSpawningSubprocess {
                path: ffmpeg_path.to_path_buf(),
                args: args.to_vec(),
                source,
            })?;

        let mut stdout: ChildStdout = ffmpeg_process.stdout.take().expect("stdout was piped");

        let mut raw = vec![0u8; READ_BUFFER_SIZE];
        let mut samples: Vec<i16> = Vec::with_capacity(READ_BUFFER_SIZE / 2);
        // a read can end in the middle of a sample, so the odd byte waits here
        let mut half_sample: Option<u8> = None;
        let mut progress_prescaler_counter = 0;

        loop {
            let read_bytes = stdout.read(&mut raw).map_err(DecoderError::ReadError)?;

            if read_bytes == 0 {
                break;
            }

            let mut bytes = &raw[..read_bytes];
            samples.clear();

            if let Some(first_byte) = half_sample.take() {
                if let Some((&second_byte, rest)) = bytes.split_first() {
                    samples.push(i16::from_le_bytes([first_byte, second_byte]));
                    bytes = rest;
                } else {
                    half_sample = Some(first_byte);
                }
            }

            let mut sample_pairs = bytes.chunks_exact(2);
            samples.extend(sample_pairs.by_ref().map(|pair| i16::from_le_bytes([pair[0], pair[1]])));
            if let [last_byte] = *sample_pairs.remainder() {
                half_sample = Some(last_byte);
            }

            receiver
                .push_samples(&samples)
                .map_err(|source| DecoderError::AudioSegmentProcessingFailed(Box::new(source)))?;

            progress_prescaler_counter += samples.len() as i64;
            while progress_prescaler_counter >= PROGRESS_PRESCALER {
                progress_handler.inc();
                progress_prescaler_counter -= PROGRESS_PRESCALER;
            }
        }

        let exit_code = ffmpeg_process
            .wait()
            .map_err(|source| DecoderError::WaitingForProcessFailed {
                cmd_path: ffmpeg_path.to_path_buf(),
                source,
            })?
            .code();

        if exit_code != Some(0) {
            return Err(Self::process_error(
                ffmpeg_path,
                exit_code,
                ffmpeg_process.stderr.expect("stderr was piped"),
            ));
        }

        progress_handler.finish();
        receiver
            .finish()
            .map_err(|source| DecoderError::AudioSegmentProcessingFailed(Box::new(source)))
    }

    /// Reads whatever the process wrote to stderr and pairs it with its exit code.
    fn process_error(cmd_path: &Path, code: Option<i32>, mut stderr: impl Read) -> DecoderError {
        let mut stderr_data = Vec::new();
        if let Err(source) = stderr.read_to_end(&mut stderr_data) {
            return DecoderError::ReadError(source);
        }

        Self::process_error_from_stderr(cmd_path, code, &String::from_utf8_lossy(&stderr_data))
    }

    fn process_error_from_stderr(cmd_path: &Path, code: Option<i32>, stderr: &str) -> DecoderError {
        DecoderError::ProcessErrorCode {
            cmd_path: cmd_path.to_path_buf(),
            code,
            source: if stderr.is_empty() {
                None
            } else {
                Some(Box::new(DecoderError::ProcessErrorMessage { msg: stderr.to_owned() }))
            },
        }
    }

    fn get_metadata(file_path: &Path, ffprobe_path: &Path, args: &[OsString]) -> Result<Metadata, DecoderError> {
        let ffprobe_process: Output = Command::new(ffprobe_path)
            .args(args)
            .stdin(Stdio::null())
            .stderr(Stdio::piped())
            .stdout(Stdio::piped())
            .output()
            .map_err(|source| DecoderError::FailedSpawningSubprocess {
                path: ffprobe_path.to_path_buf(),
                args: args.to_vec(),
                source,
            })?;

        if !ffprobe_process.status.success() {
            let stderr = String::from_utf8_lossy(&ffprobe_process.stderr);
            return Err(Self::process_error_from_stderr(
                ffprobe_path,
                ffprobe_process.status.code(),
                stderr.trim_end(),
            ));
        }

        let stdout = from_utf8(&ffprobe_process.stdout).map_err(DecoderError::FailedToDecodeVideoStreamInfo)?;

        serde_json::from_str(stdout).map_err(|source| DecoderError::DeserializingMetadataFailed {
            path: file_path.to_path_buf(),
            source,
        })
    }
}
