// This file is part of the Rust library and binary `alass`.
//
// Copyright (C) 2017 kaegi
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with this program.  If not, see <http://www.gnu.org/licenses/>.

// Alg* stands for algorithm (the internal alass algorithm types)

use alass_cli::errors::TopLevelError;
use alass_cli::*;
use alass_core::{TimeDelta as AlgTimeDelta, align, get_nosplit_score, standard_scoring};
use alass_subparse::timetypes::{TimePoint, TimeSpan};
use alass_subparse::{SubtitleEntry, SubtitleFileInterface, SubtitleFormat};
use anyhow::Result;
use clap::Parser;
use clap::builder::RangedI64ValueParser;
use encoding_rs::Encoding;
use std::cmp::min;
use std::ffi::OsStr;
use std::path::PathBuf;
use std::str::FromStr;

/// Charset encoding requested on the command line: either a label `encoding_rs`
/// knows, or `auto` for automatic detection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct EncodingArg(Option<&'static Encoding>);

impl FromStr for EncodingArg {
    type Err = String;

    fn from_str(label: &str) -> Result<Self, Self::Err> {
        if label.eq_ignore_ascii_case("auto") {
            return Ok(Self(None));
        }
        Encoding::for_label_no_replacement(label.as_bytes())
            .map(|encoding| Self(Some(encoding)))
            .ok_or_else(|| format!("'{label}' is not a known encoding label"))
    }
}

/// Parses an `f64`, rejecting the NaN and infinities that `f64::from_str` accepts.
fn finite_f64(s: &str) -> Result<f64, String> {
    let value: f64 = s.parse().map_err(|_| format!("`{s}` is not a number"))?;
    value
        .is_finite()
        .then_some(value)
        .ok_or_else(|| format!("`{s}` is not a finite number"))
}

/// Builds a `value_parser` accepting a finite `f64` inside `min..=max`.
fn f64_in_range(min: f64, max: f64) -> impl Fn(&str) -> Result<f64, String> + Clone {
    move |s: &str| {
        let value = finite_f64(s)?;
        (min..=max)
            .contains(&value)
            .then_some(value)
            .ok_or_else(|| format!("value must be between {min} and {max}"))
    }
}

/// `value_parser` for a finite `f64` greater than zero.
fn positive_f64(s: &str) -> Result<f64, String> {
    let value = finite_f64(s)?;
    (value > 0.0)
        .then_some(value)
        .ok_or_else(|| "value must be greater than 0".to_owned())
}

/// `value_parser` for a finite `f64` that is not negative.
fn non_negative_f64(s: &str) -> Result<f64, String> {
    let value = finite_f64(s)?;
    (value >= 0.0)
        .then_some(value)
        .ok_or_else(|| "value must not be negative".to_owned())
}

#[derive(Parser, Debug)]
#[command(
    version,
    about,
    after_help = "This program works with .srt, .ass/.ssa, .idx and .sub files. \
                  The corrected file will have the same format as the incorrect file."
)]
struct Arguments {
    /// Path to the reference subtitle or video file
    #[arg(value_name = "REFERENCE_FILE")]
    reference_file_path: PathBuf,

    /// Path to the incorrect subtitle file
    #[arg(value_name = "INCORRECT_SUB_FILE")]
    incorrect_file_path: PathBuf,

    /// Path to corrected subtitle file
    #[arg(value_name = "OUTPUT_FILE")]
    output_file_path: PathBuf,

    /// How eager the algorithm is to avoid splitting the subtitles [0 to 1000]
    ///
    /// 1000 means that all lines will be shifted by the same offset, while 0.01 will produce
    /// MANY segments with different offsets. Values from 1 to 20 are the most useful.
    #[arg(
        short = 'p',
        long,
        value_name = "PENALTY",
        default_value_t = 7.0,
        value_parser = f64_in_range(0.0, 1000.0),
    )]
    split_penalty: f64,

    /// Smallest recognized time interval in milliseconds
    ///
    /// Smaller numbers make the alignment more accurate, greater numbers make aligning faster.
    #[arg(
        short = 'i',
        long,
        value_name = "MILLISECONDS",
        default_value_t = 1,
        value_parser = RangedI64ValueParser::<i64>::new().range(1..),
    )]
    interval: i64,

    /// Write negative timestamps to the output file instead of clamping them to zero
    ///
    /// Negative timestamps can lead to problems with the output file, so by default 0 will be
    /// written instead. This option allows you to disable this behavior.
    #[arg(short = 'n', long)]
    allow_negative_timestamps: bool,

    /// Frames-per-second of the video accompanying a MicroDVD `.sub` reference file
    ///
    /// MicroDVD `.sub` files store their timing information as frame numbers, so they can only
    /// be read with the framerate of the video they were made for.
    #[arg(long, value_name = "FPS", default_value_t = 30.0, value_parser = positive_f64)]
    sub_fps_ref: f64,

    /// Frames-per-second of the video accompanying a MicroDVD `.sub` input file
    ///
    /// MicroDVD `.sub` files store their timing information as frame numbers, so they can only
    /// be read with the framerate of the video they were made for.
    #[arg(long, value_name = "FPS", default_value_t = 30.0, value_parser = positive_f64)]
    sub_fps_inc: f64,

    /// Charset encoding of the reference subtitle file
    #[arg(long, value_name = "ENCODING", default_value = "auto")]
    encoding_ref: EncodingArg,

    /// Charset encoding of the incorrect subtitle file
    #[arg(long, value_name = "ENCODING", default_value = "auto")]
    encoding_inc: EncodingArg,

    /// Trade accuracy for speed; 0 disables the optimization
    #[arg(
        short = 'O',
        long,
        value_name = "FACTOR",
        default_value_t = 1.0,
        value_parser = non_negative_f64,
    )]
    speed_optimization: f64,

    /// Synchronize without looking for splits/breaks - this mode is much faster
    #[arg(short = 'l', long)]
    no_split: bool,

    /// Disable guessing and correcting of framerate differences between the two files
    #[arg(short = 'g', long, alias = "disable-framerate-guessing")]
    disable_fps_guessing: bool,

    /// Audio stream index inside the reference video file
    #[arg(long = "index", value_name = "INDEX")]
    audio_index: Option<usize>,

    /// Exit with code 2 if the alignment score is below this value [0 to 1]
    ///
    /// The output file is still written, so it can be inspected. The score is always printed,
    /// with or without this option. Pick a threshold by running a few files you trust first -
    /// the attainable score depends on how similarly the two files split their lines. Note that
    /// the score measures overlap and the algorithm maximizes overlap, so it reliably catches a
    /// result that found nothing to match, but not one that matched the wrong thing; check the
    /// reported shift range for that.
    #[arg(long, value_name = "SCORE", value_parser = f64_in_range(0.0, 1.0))]
    min_score: Option<f64>,
}

impl Arguments {
    /// `None` disables the speed optimization.
    fn speed_optimization(&self) -> Option<f64> {
        (self.speed_optimization > 0.0).then_some(self.speed_optimization)
    }

    fn guess_fps_ratio(&self) -> bool {
        !self.disable_fps_guessing
    }
}

fn prepare_reference_file(args: &Arguments) -> Result<InputFileHandler> {
    let mut ref_file = InputFileHandler::open(
        &args.reference_file_path,
        args.audio_index,
        args.encoding_ref.0,
        args.sub_fps_ref,
        ProgressInfo::new(
            500,
            Some(format!(
                "extracting audio from reference file '{}'...",
                args.reference_file_path.display()
            )),
        )
        .with_steady_tick(),
    )?;

    ref_file.filter_video_with_min_span_length_ms(500);

    Ok(ref_file)
}

/// Writes a `.srt` file holding nothing but the timings of the reference file, which is
/// what `alass <reference> _ <output>` is for: it makes the voice activity detection
/// visible in a subtitle player.
fn write_reference_timings_as_srt(args: &Arguments) -> Result<()> {
    let ref_file = prepare_reference_file(args)?;

    eprintln!("input file path was given as '_'");
    eprintln!("the output file is a .srt file only containing timing information from the reference file");
    eprintln!("this can be used as a debugging tool");
    eprintln!();

    let lines: Vec<(TimeSpan, String)> = ref_file
        .timespans()
        .iter()
        .enumerate()
        .map(|(i, &time_span)| (time_span, format!("line {i}")))
        .collect();

    let debug_file = alass_subparse::SrtFile::create(lines).map_err(TopLevelError::FailedToInstantiateSubtitleFile)?;

    let data = debug_file
        .to_data()
        .map_err(TopLevelError::FailedToGenerateSubtitleData)?;

    write_data_to_file(&args.output_file_path, &data)?;

    Ok(())
}

/// Returns the process exit code: 0 on success, 2 when the alignment score is
/// below the `--min-score` threshold requested by the user.
fn run(args: Arguments) -> Result<i32> {
    if args.incorrect_file_path == OsStr::new("_") {
        // DEBUG MODE FOR REFERENCE FILE WAS ACTIVATED
        write_reference_timings_as_srt(&args)?;
        return Ok(0);
    }

    // open incorrect file before reference file so that incorrect-file-not-found-errors are
    // not displayed after the long audio extraction
    let inc_file = SubtitleFileHandler::open_sub_file(
        args.incorrect_file_path.as_path(),
        args.encoding_inc.0,
        args.sub_fps_inc,
    )?;

    let ref_file = prepare_reference_file(&args)?;

    let output_file_format = inc_file.file_format();

    // this program internally stores the files in a non-destructable way (so
    // formatting is preserved) but has no abilty to convert between formats
    if !alass_subparse::is_valid_extension_for_subtitle_format(args.output_file_path.extension(), output_file_format) {
        return Err(TopLevelError::FileFormatMismatch {
            input_file_path: args.incorrect_file_path,
            output_file_path: args.output_file_path,
            input_file_format: output_file_format,
        }
        .into());
    }

    let mut inc_aligner_timespans: Vec<alass_core::TimeSpan> =
        timings_to_alg_timespans(inc_file.timespans(), args.interval);
    let ref_aligner_timespans: Vec<alass_core::TimeSpan> =
        timings_to_alg_timespans(ref_file.timespans(), args.interval);

    let mut fps_scaling_factor = 1.;
    if args.guess_fps_ratio() {
        let a = 25.;
        let b = 24.;
        let c = 23.976;
        let ratios = [a / b, a / c, b / a, b / c, c / a, c / b];
        let desc = ["25/24", "25/23.976", "24/25", "24/23.976", "23.976/25", "23.976/24"];

        let (opt_ratio_idx, _) = guess_fps_ratio(
            &ref_aligner_timespans,
            &inc_aligner_timespans,
            &ratios,
            ProgressInfo::new(1, Some("Guessing framerate ratio...".to_string())),
        );

        fps_scaling_factor = opt_ratio_idx.map_or(1., |idx| ratios[idx]);

        println!(
            "info: 'reference file FPS/input file FPS' ratio is {}",
            opt_ratio_idx.map_or("1", |idx| desc[idx])
        );
        println!();

        inc_aligner_timespans = inc_aligner_timespans
            .into_iter()
            .map(|x| x.scaled(fps_scaling_factor))
            .collect();
    }

    let align_start_msg = format!(
        "synchronizing '{}' to reference file '{}'...",
        args.incorrect_file_path.display(),
        args.reference_file_path.display()
    );

    let alg_deltas = if args.no_split {
        let alg_delta = alass_core::align_nosplit(
            &ref_aligner_timespans,
            &inc_aligner_timespans,
            standard_scoring,
            ProgressInfo::new(1, Some(align_start_msg)),
        )
        .0;

        vec![alg_delta; inc_aligner_timespans.len()]
    } else {
        align(
            &ref_aligner_timespans,
            &inc_aligner_timespans,
            args.split_penalty,
            args.speed_optimization(),
            standard_scoring,
            ProgressInfo::new(1, Some(align_start_msg)),
        )
        .0
    };
    let deltas = alg_deltas_to_timing_deltas(&alg_deltas, args.interval);

    // How well does the result actually match the reference? This is the plain overlap
    // rating of the aligned spans; it deliberately leaves out the split penalty, so the
    // number stays comparable between runs that used a different `--split-penalty`.
    //
    // `align_with_splits` notes that a single span can contribute at most 1 to the
    // rating, so `min(ref_len, in_len)` is the highest attainable total - dividing by it
    // puts the score on a 0 to 1 scale.
    let max_attainable_rating = min(ref_aligner_timespans.len(), inc_aligner_timespans.len());
    let alignment_score: f64 = if max_attainable_rating == 0 {
        0.0
    } else {
        get_nosplit_score(
            ref_aligner_timespans.iter().copied(),
            inc_aligner_timespans
                .iter()
                .zip(alg_deltas.iter())
                .map(|(&timespan, &delta)| timespan + delta),
            standard_scoring,
        ) / max_attainable_rating as f64
    };

    // group subtitles lines which have the same offset
    let shift_groups: Vec<(AlgTimeDelta, Vec<TimeSpan>)> = get_subtitle_delta_groups(
        alg_deltas
            .iter()
            .copied()
            .zip(inc_file.timespans().iter().copied())
            .collect(),
    );

    let block_count = shift_groups.len();

    for (shift_group_delta, shift_group_lines) in shift_groups {
        // computes the first and last timestamp for all lines with that delta
        // -> that way we can provide the user with an information like
        //     "100 subtitles with 10min length"
        let first = shift_group_lines
            .iter()
            .map(|subline| subline.start)
            .min()
            .expect("a subtitle group should have at least one subtitle line");
        let last = shift_group_lines
            .iter()
            .map(|subline| subline.start)
            .max()
            .expect("a subtitle group should have at least one subtitle line");

        println!(
            "shifted block of {} subtitles with length {} by {}",
            shift_group_lines.len(),
            last - first,
            alg_delta_to_delta(shift_group_delta, args.interval)
        );
    }

    println!();
    println!("alignment score: {alignment_score:.3} (0 = no overlap, 1 = perfect overlap)");

    // The score above measures overlap, and the algorithm picks its result by maximizing
    // overlap - so a result that is wrong *because* it chased overlap still scores well.
    // The spread between the smallest and largest shift is an independent signal: it is
    // bounded by how much the two versions really differ, which the user usually knows.
    if let (Some(&min_delta), Some(&max_delta)) = (alg_deltas.iter().min(), alg_deltas.iter().max()) {
        println!(
            "shift range: {} to {} across {block_count} block(s)",
            alg_delta_to_delta(min_delta, args.interval),
            alg_delta_to_delta(max_delta, args.interval),
        );
    }
    println!();

    if ref_file.timespans().is_empty() {
        eprintln!("warn: reference file has no subtitle lines");
        eprintln!();
    }
    if inc_file.timespans().is_empty() {
        eprintln!("warn: file with incorrect subtitles has no lines");
        eprintln!();
    }

    fn scaled_timespan(ts: TimeSpan, fps_scaling_factor: f64) -> TimeSpan {
        TimeSpan::new(
            TimePoint::from_msecs((ts.start.msecs() as f64 * fps_scaling_factor) as i64),
            TimePoint::from_msecs((ts.end.msecs() as f64 * fps_scaling_factor) as i64),
        )
    }

    let mut corrected_timespans: Vec<TimeSpan> = inc_file
        .timespans()
        .iter()
        .zip(deltas.iter())
        .map(|(&timespan, &delta)| scaled_timespan(timespan, fps_scaling_factor) + delta)
        .collect();

    if corrected_timespans.iter().any(|ts| ts.start.is_negative()) {
        eprintln!("warn: some subtitles now have negative timings, which can cause invalid subtitle files");
        if args.allow_negative_timestamps {
            eprintln!(
                "warn: negative timestamps will be written to file, because you passed '-n' or '--allow-negative-timestamps'",
            );
        } else {
            eprintln!(
                "warn: negative subtitles will therefore moved to the start of the subtitle file by default; pass '-n' or '--allow-negative-timestamps' to disable this behavior",
            );

            for corrected_timespan in &mut corrected_timespans {
                if corrected_timespan.start.is_negative() {
                    let offset = TimePoint::from_secs(0) - corrected_timespan.start;
                    corrected_timespan.start += offset;
                    corrected_timespan.end += offset;
                }
            }
        }
        eprintln!();
    }

    // .idx only has start timepoints (the subtitle is shown until the next subtitle starts) - so retiming with gaps might
    // produce errors
    if output_file_format == SubtitleFormat::VobSubIdx {
        eprintln!("warn: writing to an '.idx' file can lead to unexpected results due to restrictions of this format");
    }

    // incorrect file -> correct file
    let shifted_timespans: Vec<SubtitleEntry> = corrected_timespans.into_iter().map(SubtitleEntry::from).collect();

    // write corrected files
    let mut correct_file = inc_file.into_subtitle_file();
    correct_file
        .update_subtitle_entries(&shifted_timespans)
        .map_err(TopLevelError::FailedToUpdateSubtitle)?;

    let data = correct_file
        .to_data()
        .map_err(TopLevelError::FailedToGenerateSubtitleData)?;

    write_data_to_file(&args.output_file_path, &data)?;

    // The file is written either way - a low score means "check this one", not "throw it
    // away" - so the caller is told through the exit code instead.
    if let Some(min_score) = args.min_score
        && alignment_score < min_score
    {
        eprintln!("warn: alignment score {alignment_score:.3} is below the requested minimum of {min_score:.3}");
        eprintln!("warn: the output file was still written, so the result can be inspected");
        return Ok(2);
    }

    Ok(0)
}

fn main() {
    let args = Arguments::try_parse().unwrap_or_else(|error| {
        // clap exits with code 2 on a usage error, but 2 is this program's "the alignment
        // scored below --min-score" signal - a typo'd flag must not look like a badly
        // aligned subtitle.
        let _ = error.print();
        std::process::exit(if error.use_stderr() { 1 } else { 0 });
    });

    match run(args) {
        Ok(exit_code) => std::process::exit(exit_code),
        Err(error) => {
            print_error_chain(&error);
            std::process::exit(1)
        }
    }
}
