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

#![allow(unknown_lints)] // for clippy

// TODO: search for unsafe, panic, unimplemented

extern crate clap;
extern crate encoding_rs;
extern crate pbr;
extern crate subparse;

// Alg* stands for algorithm (the internal alass algorithm types)

use crate::subparse::SubtitleFileInterface;

use alass_core::{align, get_nosplit_score, standard_scoring, TimeDelta as AlgTimeDelta};
use clap::{App, Arg};
use encoding_rs::Encoding;
use failure::ResultExt;
use std::ffi::OsStr;
use std::path::PathBuf;
use std::result::Result;
use std::str::FromStr;

use subparse::timetypes::*;
use subparse::{SubtitleEntry, SubtitleFormat};

use alass_cli::errors::*;
use alass_cli::*;

/// Does reading, parsing and nice error handling for a f64 clap parameter.
fn unpack_clap_number_f64(
    matches: &clap::ArgMatches,
    parameter_name: &'static str,
) -> Result<f64, InputArgumentsError> {
    let paramter_value_str: &str = matches.value_of(parameter_name).unwrap();
    f64::from_str(paramter_value_str)
        .with_context(|_| {
            InputArgumentsErrorKind::ArgumentParseError {
                argument_name: parameter_name.to_string(),
                value: paramter_value_str.to_string(),
            }
            .into()
        })
        .map_err(|e| InputArgumentsError::from(e))
}

/// Does reading, parsing and nice error handling for an optional f64 clap parameter.
fn unpack_optional_clap_number_f64(
    matches: &clap::ArgMatches,
    parameter_name: &'static str,
) -> Result<Option<f64>, InputArgumentsError> {
    match matches.value_of(parameter_name) {
        None => Ok(None),
        Some(parameter_value_str) => f64::from_str(parameter_value_str)
            .with_context(|_| {
                InputArgumentsErrorKind::ArgumentParseError {
                    argument_name: parameter_name.to_string(),
                    value: parameter_value_str.to_string(),
                }
                .into()
            })
            .map(Some)
            .map_err(|e| InputArgumentsError::from(e)),
    }
}

/// Does reading, parsing and nice error handling for a f64 clap parameter.
fn unpack_clap_number_i64(
    matches: &clap::ArgMatches,
    parameter_name: &'static str,
) -> Result<i64, InputArgumentsError> {
    let paramter_value_str: &str = matches.value_of(parameter_name).unwrap();
    i64::from_str(paramter_value_str)
        .with_context(|_| {
            InputArgumentsErrorKind::ArgumentParseError {
                argument_name: parameter_name.to_string(),
                value: paramter_value_str.to_string(),
            }
            .into()
        })
        .map_err(|e| InputArgumentsError::from(e))
}

fn unpack_optional_clap_number_usize(
    matches: &clap::ArgMatches,
    parameter_name: &'static str,
) -> Result<Option<usize>, InputArgumentsError> {
    
    match matches.value_of(parameter_name) {
        None => Ok(None),
        Some(parameter_value_str) => {
            usize::from_str(parameter_value_str)
                .with_context(|_| {
                    InputArgumentsErrorKind::ArgumentParseError {
                        argument_name: parameter_name.to_string(),
                        value: parameter_value_str.to_string(),
                    }
                    .into()
                })
                .map(|v| Some(v))
                .map_err(|e| InputArgumentsError::from(e))
        }
    }
}

pub fn get_encoding(opt: Option<&str>) -> Option<&'static Encoding> {
    match opt {
        None | Some("auto") => {
            // use automatic detection
            None
        },
        Some(label) => {
            match Encoding::for_label_no_replacement(label.as_bytes()) {
                None => {
                    // TODO: error handling
                    panic!("{} is not a known encoding label; exiting.", label);
                }
                Some(encoding) => Some(encoding),
            }
        }
    }
}

// //////////////////////////////////////////////////////////////////////////////////////////////////

struct Arguments {
    reference_file_path: PathBuf,
    incorrect_file_path: PathBuf,
    output_file_path: PathBuf,

    interval: i64,

    split_penalty: f64,

    sub_fps_inc: f64,
    sub_fps_ref: f64,

    allow_negative_timestamps: bool,

    /// having a value of `None` means autodetect encoding
    encoding_ref: Option<&'static Encoding>,
    encoding_inc: Option<&'static Encoding>,

    guess_fps_ratio: bool,
    no_split_mode: bool,
    speed_optimization: Option<f64>,

    audio_index: Option<usize>,

    /// if set, exit with a non-zero code when the alignment score falls below this value
    min_score: Option<f64>,
}

fn parse_args() -> Result<Arguments, InputArgumentsError> {
    let matches = App::new(PKG_NAME.unwrap_or("unkown (not compiled with cargo)"))
        .version(PKG_VERSION.unwrap_or("unknown (not compiled with cargo)"))
        .about(PKG_DESCRIPTION.unwrap_or("unknown (not compiled with cargo)"))
        .arg(Arg::with_name("reference-file")
            .help("Path to the reference subtitle or video file")
            .required(true))
        .arg(Arg::with_name("incorrect-sub-file")
            .help("Path to the incorrect subtitle file")
            .required(true))
        .arg(Arg::with_name("output-file-path")
            .help("Path to corrected subtitle file")
            .required(true))
        .arg(Arg::with_name("split-penalty")
            .short("p")
            .long("split-penalty")
            .value_name("floating point number from 0 to 1000")
            .help("Determines how eager the algorithm is to avoid splitting of the subtitles. 1000 means that all lines will be shifted by the same offset, while 0.01 will produce MANY segments with different offsets. Values from 1 to 20 are the most useful.")
            .default_value("7"))
        .arg(Arg::with_name("interval")
            .short("i")
            .long("interval")
            .value_name("integer in milliseconds")
            .help("The smallest recognized time interval, smaller numbers make the alignment more accurate, greater numbers make aligning faster.")
            .default_value("1"))
        .arg(Arg::with_name("allow-negative-timestamps")
            .short("n")
            .long("allow-negative-timestamps")
            .help("Negative timestamps can lead to problems with the output file, so by default 0 will be written instead. This option allows you to disable this behavior."))
        .arg(Arg::with_name("sub-fps-ref")
            .long("sub-fps-ref")
            .value_name("floating-point number in frames-per-second")
            .default_value("30")
            .help("Specifies the frames-per-second for the accompanying video of MicroDVD `.sub` files (MicroDVD `.sub` files store timing information as frame numbers). Only affects the reference subtitle file."))
        .arg(Arg::with_name("sub-fps-inc")
            .long("sub-fps-inc")
            .value_name("floating-point number in frames-per-second")
            .default_value("30")
            .help("Specifies the frames-per-second for the accompanying video of MicroDVD `.sub` files (MicroDVD `.sub` files store timing information as frame numbers). Only affects the incorrect subtitle file."))
        .arg(Arg::with_name("encoding-ref")
            .long("encoding-ref")
            .value_name("encoding")
            .help("Charset encoding of the reference subtitle file.")
            .default_value("auto"))
        .arg(Arg::with_name("encoding-inc")
            .long("encoding-inc")
            .value_name("encoding")
            .help("Charset encoding of the incorrect subtitle file.")
            .default_value("auto"))
        .arg(Arg::with_name("speed-optimization")
            .long("speed-optimization")
            .short("O")
            .value_name("path")
            .default_value("1")
            .help("(greatly) speeds up synchronization by sacrificing some accuracy; set to 0 to disable speed optimization")
            .required(false)
            )
        .arg(Arg::with_name("statistics-required-tag")
            .long("statistics-required-tag")
            .short("t")
            .value_name("tag")
            .help("only output statistics containing this tag (you can find the tags in statistics file)")
            .required(false)
            )
        .arg(Arg::with_name("no-split")
            .help("synchronize subtitles without looking for splits/breaks - this mode is much faster")
            .short("l")
            .long("no-split")
        )
        .arg(Arg::with_name("disable-fps-guessing")
            .help("disables guessing and correcting of framerate differences between reference file and input file")
            .short("g")
            .long("disable-fps-guessing")
            .alias("disable-framerate-guessing")
        )
        .arg(Arg::with_name("audio-index")
            .help("specifies the audio index in the reference video file")
            .long("index")
            .value_name("audio-index")
            .required(false)
        )
        .arg(Arg::with_name("min-score")
            .long("min-score")
            .value_name("score between 0 and 1")
            .required(false)
            .help("Exit with code 2 if the alignment score is below this value; the output file is still written, so it can be inspected. The score is always printed, with or without this option. Pick a threshold by running a few files you trust first - the attainable score depends on how similarly the two files split their lines. Note that the score measures overlap and the algorithm maximizes overlap, so it reliably catches a result that found nothing to match, but not one that matched the wrong thing; check the reported shift range for that."))
        .after_help("This program works with .srt, .ass/.ssa, .idx and .sub files. The corrected file will have the same format as the incorrect file.")
        .get_matches();

    let reference_file_path: PathBuf = matches.value_of("reference-file").unwrap().into();
    let incorrect_file_path: PathBuf = matches.value_of("incorrect-sub-file").unwrap().into();
    let output_file_path: PathBuf = matches.value_of("output-file-path").unwrap().into();

    let interval: i64 = unpack_clap_number_i64(&matches, "interval")?;
    if interval < 1 {
        return Err(InputArgumentsErrorKind::ExpectedPositiveNumber {
            argument_name: "interval".to_string(),
            value: interval,
        }
        .into());
    }

    let split_penalty: f64 = unpack_clap_number_f64(&matches, "split-penalty")?;
    if split_penalty < 0.0 || split_penalty > 1000.0 {
        return Err(InputArgumentsErrorKind::ValueNotInRange {
            argument_name: "interval".to_string(),
            value: split_penalty,
            min: 0.0,
            max: 1000.0,
        }
        .into());
    }

    let speed_optimization: f64 = unpack_clap_number_f64(&matches, "speed-optimization")?;
    if split_penalty < 0.0 {
        return Err(InputArgumentsErrorKind::ExpectedNonNegativeNumber {
            argument_name: "speed-optimization".to_string(),
            value: speed_optimization,
        }
        .into());
    }

    let no_split_mode: bool = matches.is_present("no-split");

    Ok(Arguments {
        reference_file_path,
        incorrect_file_path,
        output_file_path,
        interval,
        split_penalty,
        sub_fps_ref: unpack_clap_number_f64(&matches, "sub-fps-ref")?,
        sub_fps_inc: unpack_clap_number_f64(&matches, "sub-fps-inc")?,
        allow_negative_timestamps: matches.is_present("allow-negative-timestamps"),
        encoding_ref: get_encoding(matches.value_of("encoding-ref")),
        encoding_inc: get_encoding(matches.value_of("encoding-inc")),
        no_split_mode,
        guess_fps_ratio: !matches.is_present("disable-fps-guessing"),
        speed_optimization: if speed_optimization <= 0. {
            None
        } else {
            Some(speed_optimization)
        },
        audio_index: unpack_optional_clap_number_usize(&matches, "audio-index")?,
        min_score: {
            let min_score = unpack_optional_clap_number_f64(&matches, "min-score")?;
            if let Some(value) = min_score {
                if value < 0.0 || value > 1.0 {
                    return Err(InputArgumentsErrorKind::ValueNotInRange {
                        argument_name: "min-score".to_string(),
                        value,
                        min: 0.0,
                        max: 1.0,
                    }
                    .into());
                }
            }
            min_score
        },
    })
}

fn prepare_reference_file(args: &Arguments) -> Result<InputFileHandler, failure::Error> {
    let mut ref_file = InputFileHandler::open(
        &args.reference_file_path,
        args.audio_index,
        args.encoding_ref,
        args.sub_fps_ref,
        ProgressInfo::new(
            500,
            Some(format!(
                "extracting audio from reference file '{}'...",
                args.reference_file_path.display()
            )),
        ),
    )?;

    ref_file.filter_video_with_min_span_length_ms(500);

    Ok(ref_file)
}

// //////////////////////////////////////////////////////////////////////////////////////////////////

/// Returns the process exit code: 0 on success, 2 when the alignment score is
/// below the `--min-score` threshold requested by the user.
fn run() -> Result<i32, failure::Error> {
    let args = parse_args()?;

    if args.incorrect_file_path.eq(OsStr::new("_")) {
        // DEBUG MODE FOR REFERENCE FILE WAS ACTIVATED
        let ref_file = prepare_reference_file(&args)?;

        eprintln!("input file path was given as '_'");
        eprintln!("the output file is a .srt file only containing timing information from the reference file");
        eprintln!("this can be used as a debugging tool");
        eprintln!();

        let lines: Vec<(subparse::timetypes::TimeSpan, String)> = ref_file
            .timespans()
            .iter()
            .cloned()
            .enumerate()
            .map(|(i, time_span)| (time_span, format!("line {}", i)))
            .collect();

        let debug_file =
            subparse::SrtFile::create(lines).with_context(|_| TopLevelErrorKind::FailedToInstantiateSubtitleFile)?;

        write_data_to_file(
            &args.output_file_path,
            debug_file.to_data().unwrap(), // error handling
        )?;

        return Ok(0);
    }

    // open incorrect file before reference file before so that incorrect-file-not-found-errors are not displayed after the long audio extraction
    let inc_file =
        SubtitleFileHandler::open_sub_file(args.incorrect_file_path.as_path(), args.encoding_inc, args.sub_fps_inc)?;

    let ref_file = prepare_reference_file(&args)?;

    let output_file_format = inc_file.file_format();

    // this program internally stores the files in a non-destructable way (so
    // formatting is preserved) but has no abilty to convert between formats
    if !subparse::is_valid_extension_for_subtitle_format(args.output_file_path.extension(), output_file_format) {
        return Err(TopLevelErrorKind::FileFormatMismatch {
            input_file_path: args.incorrect_file_path,
            output_file_path: args.output_file_path,
            input_file_format: inc_file.file_format(),
        }
        .into_error()
        .into());
    }

    let mut inc_aligner_timespans: Vec<alass_core::TimeSpan> =
        timings_to_alg_timespans(inc_file.timespans(), args.interval);
    let ref_aligner_timespans: Vec<alass_core::TimeSpan> =
        timings_to_alg_timespans(ref_file.timespans(), args.interval);

    let mut fps_scaling_factor = 1.;
    if args.guess_fps_ratio {
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

        fps_scaling_factor = if let Some(idx) = opt_ratio_idx { ratios[idx] } else { 1. };

        println!(
            "info: 'reference file FPS/input file FPS' ratio is {}",
            if let Some(idx) = opt_ratio_idx { desc[idx] } else { "1" }
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
    let alg_deltas;
    if args.no_split_mode {
        let num_inc_timespancs = inc_aligner_timespans.len();

        let alg_delta = alass_core::align_nosplit(
            &ref_aligner_timespans,
            &inc_aligner_timespans,
            alass_core::standard_scoring,
            ProgressInfo::new(1, Some(align_start_msg)),
        )
        .0;

        alg_deltas = std::vec::from_elem(alg_delta, num_inc_timespancs);
    } else {
        alg_deltas = align(
            &ref_aligner_timespans,
            &inc_aligner_timespans,
            args.split_penalty,
            args.speed_optimization,
            alass_core::standard_scoring,
            ProgressInfo::new(1, Some(align_start_msg)),
        )
        .0;
    }
    let deltas = alg_deltas_to_timing_deltas(&alg_deltas, args.interval);

    // How well does the result actually match the reference? This is the plain overlap
    // rating of the aligned spans; it deliberately leaves out the split penalty, so the
    // number stays comparable between runs that used a different `--split-penalty`.
    //
    // `align_with_splits` notes that a single span can contribute at most 1 to the
    // rating, so `min(ref_len, in_len)` is the highest attainable total - dividing by it
    // puts the score on a 0 to 1 scale.
    let max_attainable_rating = std::cmp::min(ref_aligner_timespans.len(), inc_aligner_timespans.len());
    let alignment_score: f64 = if max_attainable_rating == 0 {
        0.0
    } else {
        get_nosplit_score(
            ref_aligner_timespans.iter().cloned(),
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
            .cloned()
            .zip(inc_file.timespans().iter().cloned())
            .collect(),
    );

    let block_count = shift_groups.len();

    for (shift_group_delta, shift_group_lines) in shift_groups {
        // computes the first and last timestamp for all lines with that delta
        // -> that way we can provide the user with an information like
        //     "100 subtitles with 10min length"
        let min = shift_group_lines
            .iter()
            .map(|subline| subline.start)
            .min()
            .expect("a subtitle group should have at least one subtitle line");
        let max = shift_group_lines
            .iter()
            .map(|subline| subline.start)
            .max()
            .expect("a subtitle group should have at least one subtitle line");

        println!(
            "shifted block of {} subtitles with length {} by {}",
            shift_group_lines.len(),
            max - min,
            alg_delta_to_delta(shift_group_delta, args.interval)
        );
    }

    println!();
    println!("alignment score: {:.3} (0 = no overlap, 1 = perfect overlap)", alignment_score);

    // The score above measures overlap, and the algorithm picks its result by maximizing
    // overlap - so a result that is wrong *because* it chased overlap still scores well.
    // The spread between the smallest and largest shift is an independent signal: it is
    // bounded by how much the two versions really differ, which the user usually knows.
    if let (Some(&min_delta), Some(&max_delta)) = (alg_deltas.iter().min(), alg_deltas.iter().max()) {
        println!(
            "shift range: {} to {} across {} block(s)",
            alg_delta_to_delta(min_delta, args.interval),
            alg_delta_to_delta(max_delta, args.interval),
            block_count
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

    let mut corrected_timespans: Vec<subparse::timetypes::TimeSpan> = inc_file
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
                    let offset = subparse::timetypes::TimePoint::from_secs(0) - corrected_timespan.start;
                    corrected_timespan.start = corrected_timespan.start + offset;
                    corrected_timespan.end = corrected_timespan.end + offset;
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
    let shifted_timespans: Vec<SubtitleEntry> = corrected_timespans
        .into_iter()
        .map(|timespan| SubtitleEntry::from(timespan))
        .collect();

    // write corrected files
    let mut correct_file = inc_file.into_subtitle_file();
    correct_file
        .update_subtitle_entries(&shifted_timespans)
        .with_context(|_| TopLevelErrorKind::FailedToUpdateSubtitle)?;

    write_data_to_file(
        &args.output_file_path,
        correct_file
            .to_data()
            .with_context(|_| TopLevelErrorKind::FailedToGenerateSubtitleData)?,
    )?;

    // The file is written either way - a low score means "check this one", not "throw it
    // away" - so the caller is told through the exit code instead.
    if let Some(min_score) = args.min_score {
        if alignment_score < min_score {
            eprintln!(
                "warn: alignment score {:.3} is below the requested minimum of {:.3}",
                alignment_score, min_score
            );
            eprintln!("warn: the output file was still written, so the result can be inspected");
            return Ok(2);
        }
    }

    Ok(0)
}

// //////////////////////////////////////////////////////////////////////////////////////////////////

fn main() {
    match run() {
        Ok(exit_code) => std::process::exit(exit_code),
        Err(error) => {
            print_error_chain(error);
            std::process::exit(1)
        }
    }
}
