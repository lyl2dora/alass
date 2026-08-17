use alass_core::{TimeDelta as AlgTimeDelta, TimePoint as AlgTimePoint, TimeSpan as AlgTimeSpan};
use alass_subparse::timetypes::{TimeDelta, TimePoint, TimeSpan};
use alass_subparse::{SubtitleFile, SubtitleFormat, get_subtitle_format_err, parse_bytes};
use encoding_rs::Encoding;
use indicatif::{ProgressBar, ProgressDrawTarget, ProgressStyle};
use std::cmp::{max, min};
use std::ffi::OsStr;
use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use std::time::Duration;

use errors::{FileOperationError, InputFileError, InputSubtitleError, InputVideoError};

pub mod errors;
pub mod video_decoder;

/// Layout of the progress bar: `12 / 34 [=====>-----]  35 % 1.2/s 4s`.
static PROGRESS_STYLE: LazyLock<ProgressStyle> = LazyLock::new(|| {
    ProgressStyle::with_template("{pos} / {len} [{wide_bar}] {percent:>3} % {per_sec} {eta}")
        .expect("the progress bar template is a literal and always parses")
        .progress_chars("=>-")
});

/// Does not report progress at all.
#[derive(Debug)]
pub struct NoProgressInfo;

impl alass_core::ProgressHandler for NoProgressInfo {}
impl video_decoder::ProgressHandler for NoProgressInfo {}

/// Draws a progress bar on stderr, but only when stderr is a terminal.
#[derive(Debug)]
pub struct ProgressInfo {
    init_msg: Option<String>,
    prescaler: i64,
    counter: i64,
    steady_tick: bool,
    progress_bar: Option<ProgressBar>,
}

impl ProgressInfo {
    pub fn new(prescaler: i64, init_msg: Option<String>) -> Self {
        assert!(prescaler > 0, "the progress prescaler is used as a divisor");
        Self {
            init_msg,
            prescaler,
            counter: 0,
            steady_tick: false,
            progress_bar: None,
        }
    }

    /// Redraws the bar on a timer instead of only when progress arrives.
    ///
    /// Worth it for a phase driven by an external process - a slow `ffmpeg` should still
    /// look alive. Not worth it for the short in-process phases: the ticker draws from its
    /// own thread and reads the position independently of the thread incrementing it, so
    /// a fast bar renders frames whose count, percentage and filled width disagree.
    #[must_use]
    pub fn with_steady_tick(mut self) -> Self {
        self.steady_tick = true;
        self
    }

    fn init(&mut self, steps: i64) {
        // Printed before the bar exists: the steady tick redraws from a background
        // thread and would otherwise race with this line.
        if let Some(init_msg) = &self.init_msg {
            eprintln!("{init_msg}");
        }

        // A progress bar redraws itself with carriage returns, which turns into
        // thousands of junk lines once the output is a pipe or a log file - so it is
        // only drawn for an interactive terminal. It also belongs on stderr, so that
        // stdout carries nothing but the alignment report. `ProgressDrawTarget::stderr`
        // makes both decisions itself and yields a hidden target when either fails.
        let bar = ProgressBar::with_draw_target(
            Some((steps / self.prescaler).max(0) as u64),
            ProgressDrawTarget::stderr(),
        );
        if bar.is_hidden() {
            return;
        }
        bar.set_style(PROGRESS_STYLE.clone());
        if self.steady_tick {
            bar.enable_steady_tick(Duration::from_millis(100));
        }
        self.progress_bar = Some(bar);
    }

    fn inc(&mut self) {
        self.counter += 1;
        if self.counter == self.prescaler {
            if let Some(progress_bar) = &self.progress_bar {
                progress_bar.inc(1);
            }
            self.counter = 0;
        }
    }

    fn finish(&mut self) {
        // Taken out of the option so that a second `finish()` - or the `Drop` glue -
        // cannot draw the bar again.
        if let Some(progress_bar) = self.progress_bar.take() {
            progress_bar.finish();
            // `indicatif` leaves the cursor at the end of the bar line: one newline to
            // close that line, one to separate the bar from what comes next.
            eprintln!();
            eprintln!();
        }
    }
}

impl alass_core::ProgressHandler for ProgressInfo {
    fn init(&mut self, steps: i64) {
        self.init(steps);
    }
    fn inc(&mut self) {
        self.inc();
    }
    fn finish(&mut self) {
        self.finish();
    }
}

impl video_decoder::ProgressHandler for ProgressInfo {
    fn init(&mut self, steps: i64) {
        self.init(steps);
    }
    fn inc(&mut self) {
        self.inc();
    }
    fn finish(&mut self) {
        self.finish();
    }
}

pub fn read_file_to_bytes(path: &Path) -> Result<Vec<u8>, FileOperationError> {
    let mut file = File::open(path).map_err(|source| FileOperationError::FileOpen {
        path: path.to_path_buf(),
        source,
    })?;
    let mut buffer = Vec::new();
    file.read_to_end(&mut buffer)
        .map_err(|source| FileOperationError::FileRead {
            path: path.to_path_buf(),
            source,
        })?;
    Ok(buffer)
}

pub fn write_data_to_file(path: &Path, data: &[u8]) -> Result<(), FileOperationError> {
    let mut file = File::create(path).map_err(|source| FileOperationError::FileOpen {
        path: path.to_path_buf(),
        source,
    })?;
    file.write_all(data).map_err(|source| FileOperationError::FileWrite {
        path: path.to_path_buf(),
        source,
    })
}

pub fn timing_to_alg_timepoint(t: TimePoint, interval: i64) -> AlgTimePoint {
    assert!(interval > 0);
    AlgTimePoint::from(t.msecs() / interval)
}

pub fn alg_delta_to_delta(t: AlgTimeDelta, interval: i64) -> TimeDelta {
    assert!(interval > 0);
    let time_int: i64 = t.into();
    TimeDelta::from_msecs(time_int * interval)
}

pub fn timings_to_alg_timespans(v: &[TimeSpan], interval: i64) -> Vec<AlgTimeSpan> {
    v.iter()
        .map(|timespan| {
            AlgTimeSpan::new_safe(
                timing_to_alg_timepoint(timespan.start, interval),
                timing_to_alg_timepoint(timespan.end, interval),
            )
        })
        .collect()
}

pub fn alg_deltas_to_timing_deltas(v: &[AlgTimeDelta], interval: i64) -> Vec<TimeDelta> {
    v.iter().map(|&x| alg_delta_to_delta(x, interval)).collect()
}

/// Groups consecutive timespans with the same delta together.
pub fn get_subtitle_delta_groups(mut v: Vec<(AlgTimeDelta, TimeSpan)>) -> Vec<(AlgTimeDelta, Vec<TimeSpan>)> {
    v.sort_by_key(|(_, timespan)| min(timespan.start, timespan.end));

    let mut result: Vec<(AlgTimeDelta, Vec<TimeSpan>)> = Vec::new();

    for (delta, original_timespan) in v {
        match result.last_mut() {
            Some((last_delta, timespans)) if *last_delta == delta => timespans.push(original_timespan),
            _ => result.push((delta, vec![original_timespan])),
        }
    }

    result
}

/// The file the timings are taken from - either a subtitle file, or the voice
/// activity found in a video file.
#[derive(Debug)]
pub enum InputFileHandler {
    Subtitle(SubtitleFileHandler),
    Video(VideoFileHandler),
}

#[derive(Debug)]
pub struct SubtitleFileHandler {
    file_format: SubtitleFormat,
    subtitle_file: SubtitleFile,
    subparse_timespans: Vec<TimeSpan>,
}

impl SubtitleFileHandler {
    pub fn open_sub_file(
        file_path: &Path,
        sub_encoding: Option<&'static Encoding>,
        sub_fps: f64,
    ) -> Result<SubtitleFileHandler, InputSubtitleError> {
        let sub_data =
            read_file_to_bytes(file_path).map_err(|source| InputSubtitleError::ReadingSubtitleFileFailed {
                path: file_path.to_path_buf(),
                source,
            })?;

        let file_format = get_subtitle_format_err(file_path.extension(), &sub_data).map_err(|source| {
            InputSubtitleError::UnknownSubtitleFormat {
                path: file_path.to_path_buf(),
                source,
            }
        })?;

        let parsed_subtitle_data: SubtitleFile =
            parse_bytes(file_format, &sub_data, sub_encoding, sub_fps).map_err(|source| {
                InputSubtitleError::ParsingSubtitleFailed {
                    path: file_path.to_path_buf(),
                    source,
                }
            })?;

        let subparse_timespans: Vec<TimeSpan> = parsed_subtitle_data
            .get_subtitle_entries()
            .map_err(|source| InputSubtitleError::RetrievingSubtitleLinesFailed {
                path: file_path.to_path_buf(),
                source,
            })?
            .into_iter()
            .map(|subentry| subentry.timespan)
            .map(|timespan| TimeSpan::new(min(timespan.start, timespan.end), max(timespan.start, timespan.end)))
            .collect();

        Ok(SubtitleFileHandler {
            file_format,
            subparse_timespans,
            subtitle_file: parsed_subtitle_data,
        })
    }

    pub fn file_format(&self) -> SubtitleFormat {
        self.file_format
    }

    pub fn timespans(&self) -> &[TimeSpan] {
        self.subparse_timespans.as_slice()
    }

    pub fn into_subtitle_file(self) -> SubtitleFile {
        self.subtitle_file
    }
}

#[derive(Debug)]
pub struct VideoFileHandler {
    subparse_timespans: Vec<TimeSpan>,
}

impl VideoFileHandler {
    pub fn from_cache(timespans: Vec<TimeSpan>) -> VideoFileHandler {
        VideoFileHandler {
            subparse_timespans: timespans,
        }
    }

    pub fn open_video_file(
        file_path: &Path,
        audio_index: Option<usize>,
        video_decode_progress: impl video_decoder::ProgressHandler,
    ) -> Result<VideoFileHandler, InputVideoError> {
        use webrtc_vad::{SampleRate, Vad};

        struct WebRtcFvad {
            fvad: Vad,
            vad_buffer: Vec<bool>,
        }

        impl video_decoder::AudioReceiver for WebRtcFvad {
            type Output = Vec<bool>;
            type Error = InputVideoError;

            fn push_samples(&mut self, samples: &[i16]) -> Result<(), InputVideoError> {
                // the chunked audio receiver should only provide 10ms of 8000Hz -> 80 samples
                assert!(samples.len() == 80);

                let is_voice = self
                    .fvad
                    .is_voice_segment(samples)
                    .map_err(|()| InputVideoError::VadAnalysisFailed)?;

                self.vad_buffer.push(is_voice);

                Ok(())
            }

            fn finish(self) -> Result<Vec<bool>, InputVideoError> {
                Ok(self.vad_buffer)
            }
        }

        let vad_processor = WebRtcFvad {
            fvad: Vad::new_with_rate(SampleRate::Rate8kHz),
            vad_buffer: Vec::new(),
        };

        let chunk_processor = video_decoder::ChunkedAudioReceiver::new(80, vad_processor);

        let vad_buffer =
            video_decoder::VideoDecoder::decode(file_path, audio_index, chunk_processor, video_decode_progress)
                .map_err(|source| InputVideoError::FailedToDecode {
                    path: PathBuf::from(file_path),
                    source: Box::new(source),
                })?;

        let mut voice_segments: Vec<(i64, i64)> = Vec::new();
        let mut voice_segment_start: i64 = 0;

        let mut last_segment_end: i64 = 0;
        let mut already_saved_span = true;

        for (i, is_voice_segment) in vad_buffer.into_iter().chain(std::iter::once(false)).enumerate() {
            let i = i as i64;

            if is_voice_segment {
                last_segment_end = i;
                if already_saved_span {
                    voice_segment_start = i;
                    already_saved_span = false;
                }
            } else if !already_saved_span {
                voice_segments.push((voice_segment_start, last_segment_end));
                already_saved_span = true;
            }
        }

        let subparse_timespans: Vec<TimeSpan> = voice_segments
            .into_iter()
            .map(|(start, end)| TimeSpan::new(TimePoint::from_msecs(start * 10), TimePoint::from_msecs(end * 10)))
            .collect();

        Ok(VideoFileHandler { subparse_timespans })
    }

    pub fn filter_with_min_span_length_ms(&mut self, min_vad_span_length_ms: i64) {
        self.subparse_timespans
            .retain(|ts| ts.len() >= TimeDelta::from_msecs(min_vad_span_length_ms));
    }

    pub fn timespans(&self) -> &[TimeSpan] {
        self.subparse_timespans.as_slice()
    }
}

impl InputFileHandler {
    pub fn open(
        file_path: &Path,
        audio_index: Option<usize>,
        sub_encoding: Option<&'static Encoding>,
        sub_fps: f64,
        video_decode_progress: impl video_decoder::ProgressHandler,
    ) -> Result<InputFileHandler, InputFileError> {
        const KNOWN_SUBTITLE_ENDINGS: [&str; 6] = ["srt", "vob", "idx", "ass", "ssa", "sub"];

        let extension: Option<&OsStr> = file_path.extension();

        if KNOWN_SUBTITLE_ENDINGS
            .iter()
            .any(|ending| extension == Some(OsStr::new(ending)))
        {
            return SubtitleFileHandler::open_sub_file(file_path, sub_encoding, sub_fps)
                .map(InputFileHandler::Subtitle)
                .map_err(|source| InputFileError::SubtitleFile {
                    path: file_path.to_path_buf(),
                    source,
                });
        }

        VideoFileHandler::open_video_file(file_path, audio_index, video_decode_progress)
            .map(InputFileHandler::Video)
            .map_err(|source| InputFileError::VideoFile {
                path: file_path.to_path_buf(),
                source,
            })
    }

    pub fn into_subtitle_file(self) -> Option<SubtitleFile> {
        match self {
            InputFileHandler::Video(_) => None,
            InputFileHandler::Subtitle(sub_handler) => Some(sub_handler.subtitle_file),
        }
    }

    pub fn timespans(&self) -> &[TimeSpan] {
        match self {
            InputFileHandler::Video(video_handler) => video_handler.timespans(),
            InputFileHandler::Subtitle(sub_handler) => sub_handler.timespans(),
        }
    }

    pub fn filter_video_with_min_span_length_ms(&mut self, min_vad_span_length_ms: i64) {
        if let InputFileHandler::Video(video_handler) = self {
            video_handler.filter_with_min_span_length_ms(min_vad_span_length_ms);
        }
    }
}

/// Tries every framerate ratio in `ratios` and returns the one that aligns best,
/// or `None` when leaving the framerate alone already wins.
pub fn guess_fps_ratio(
    ref_spans: &[alass_core::TimeSpan],
    in_spans: &[alass_core::TimeSpan],
    ratios: &[f64],
    mut progress_handler: impl alass_core::ProgressHandler,
) -> (Option<usize>, alass_core::TimeDelta) {
    // one alignment for the unscaled spans, then one per ratio
    progress_handler.init(ratios.len() as i64 + 1);
    let (delta, score) = alass_core::align_nosplit(
        ref_spans,
        in_spans,
        alass_core::overlap_scoring,
        alass_core::NoProgressHandler,
    );
    progress_handler.inc();

    let (mut opt_idx, mut opt_delta, mut opt_score) = (None, delta, score);

    for (scale_factor_idx, scaling_factor) in ratios.iter().copied().enumerate() {
        let stretched_in_spans: Vec<alass_core::TimeSpan> =
            in_spans.iter().map(|ts| ts.scaled(scaling_factor)).collect();

        let (delta, score) = alass_core::align_nosplit(
            ref_spans,
            &stretched_in_spans,
            alass_core::overlap_scoring,
            alass_core::NoProgressHandler,
        );
        progress_handler.inc();

        if score > opt_score {
            opt_score = score;
            opt_idx = Some(scale_factor_idx);
            opt_delta = delta;
        }
    }

    progress_handler.finish();

    (opt_idx, opt_delta)
}

/// Prints an error and everything that caused it to stderr.
pub fn print_error_chain(error: &anyhow::Error) {
    let show_backtrace = std::env::var_os("RUST_BACKTRACE").is_some_and(|value| value != "0");

    eprintln!("error: {error}");
    if show_backtrace {
        eprintln!("stack trace: {}", error.backtrace());
    }

    for cause in error.chain().skip(1) {
        eprintln!("caused by: {cause}");
    }

    if !show_backtrace {
        eprintln!();
        eprintln!("note: run with environment variable 'RUST_BACKTRACE=1' for detailed stack traces");
    }
}
