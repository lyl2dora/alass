// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! Exercises the public API from outside the crate.
//!
//! Everything named here also exists in `subparse 0.7.0` under the same path, so this
//! file doubles as a source-compatibility guard.

use std::ffi::OsStr;

use alass_subparse::timetypes::{TimeDelta, TimePoint, TimeSpan};
use alass_subparse::{
    IdxFile, MdvdFile, SrtFile, SsaFile, SubtitleEntry, SubtitleFile, SubtitleFileInterface, SubtitleFormat, VobFile,
    get_subtitle_format, get_subtitle_format_by_extension, get_subtitle_format_by_extension_err,
    get_subtitle_format_err, is_valid_extension_for_subtitle_format, parse_bytes, parse_str,
};

const SRT: &str = "1\n00:00:10,000 --> 00:00:12,000\nline one\n\n2\n00:00:15,500 --> 00:00:17,500\nline two\n\n";

const SSA: &str = "[Events]\n\
                   Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\n\
                   Dialogue: 0,0:00:10.00,0:00:12.00,Default,,0,0,0,,line one\n";

const IDX: &str = "# VobSub index file, v7 (do not modify this line!)\n\
                   id: en, index: 0\n\
                   timestamp: 00:00:10:000, filepos: 000000000\n\
                   timestamp: 00:00:15:500, filepos: 000001000\n";

const MDVD: &str = "{300}{360}line one\n{465}{525}line two";

/// Parse, shift every entry by `delta`, write, and parse the result again.
fn round_trip(format: SubtitleFormat, data: &[u8], delta: TimeDelta) -> Vec<TimeSpan> {
    let mut file: SubtitleFile = parse_bytes(format, data, None, 30.0).unwrap();
    let entries = file.get_subtitle_entries().unwrap();

    let shifted: Vec<SubtitleEntry> = entries
        .iter()
        .map(|e| SubtitleEntry::from(e.timespan + delta))
        .collect();
    file.update_subtitle_entries(&shifted).unwrap();

    let written = file.to_data().unwrap();
    parse_bytes(format, &written, None, 30.0)
        .unwrap()
        .get_subtitle_entries()
        .unwrap()
        .into_iter()
        .map(|e| e.timespan)
        .collect()
}

fn span(start_ms: i64, end_ms: i64) -> TimeSpan {
    TimeSpan::new(TimePoint::from_msecs(start_ms), TimePoint::from_msecs(end_ms))
}

#[test]
fn srt_round_trip() {
    let delta = TimeDelta::from_msecs(1234);
    assert_eq!(
        round_trip(SubtitleFormat::SubRip, SRT.as_bytes(), delta),
        [span(11_234, 13_234), span(16_734, 18_734)]
    );
}

#[test]
fn ssa_round_trip() {
    // .ssa stores hundredths of a second, so the delta has to be a multiple of 10ms
    let delta = TimeDelta::from_msecs(1230);
    assert_eq!(
        round_trip(SubtitleFormat::SubStationAlpha, SSA.as_bytes(), delta),
        [span(11_230, 13_230)]
    );
}

#[test]
fn idx_round_trip() {
    // .idx has no end times: an entry ends when the next one starts, and the last
    // one lasts a minute
    let delta = TimeDelta::from_msecs(1234);
    assert_eq!(
        round_trip(SubtitleFormat::VobSubIdx, IDX.as_bytes(), delta),
        [span(11_234, 16_734), span(16_734, 76_734)]
    );
}

#[test]
fn microdvd_round_trip() {
    let delta = TimeDelta::from_msecs(1000);
    assert_eq!(
        round_trip(SubtitleFormat::MicroDVD, MDVD.as_bytes(), delta),
        [span(11_000, 13_000), span(16_500, 18_500)]
    );
}

#[test]
fn vobsub_sub_is_read_only() {
    let data = include_bytes!("../fixtures/example.sub");
    let file = parse_bytes(SubtitleFormat::VobSubSub, data, None, 30.0).unwrap();
    let entries = file.get_subtitle_entries().unwrap();
    assert_eq!(
        entries.iter().map(|e| e.timespan).collect::<Vec<_>>(),
        [span(49_466, 50_966), span(52_635, 55_565)]
    );
    // the original bytes come back unchanged
    assert_eq!(file.to_data().unwrap(), data.as_slice());
}

#[test]
fn format_detection() {
    assert_eq!(
        get_subtitle_format_by_extension(Some(OsStr::new("srt"))),
        Some(SubtitleFormat::SubRip)
    );
    assert_eq!(
        get_subtitle_format_by_extension_err(Some(OsStr::new("ass"))).unwrap(),
        SubtitleFormat::SubStationAlpha
    );
    assert_eq!(
        get_subtitle_format(Some(OsStr::new("sub")), MDVD.as_bytes()),
        Some(SubtitleFormat::MicroDVD)
    );
    assert_eq!(
        get_subtitle_format_err(Some(OsStr::new("sub")), include_bytes!("../fixtures/tiny.sub")).unwrap(),
        SubtitleFormat::VobSubSub
    );
    assert!(is_valid_extension_for_subtitle_format(
        Some(OsStr::new("idx")),
        SubtitleFormat::VobSubIdx
    ));
    assert_eq!(SubtitleFormat::SubRip.get_name(), ".srt (SubRip)");
}

#[test]
fn parse_str_and_the_per_format_constructors() {
    assert_eq!(
        parse_str(SubtitleFormat::SubRip, SRT, 30.0)
            .unwrap()
            .get_subtitle_entries()
            .unwrap()
            .len(),
        2
    );

    assert_eq!(SrtFile::parse(SRT).unwrap().get_subtitle_entries().unwrap().len(), 2);
    assert_eq!(SsaFile::parse(SSA).unwrap().get_subtitle_entries().unwrap().len(), 1);
    assert_eq!(IdxFile::parse(IDX).unwrap().get_subtitle_entries().unwrap().len(), 2);
    assert_eq!(
        MdvdFile::parse(MDVD, 30.0)
            .unwrap()
            .get_subtitle_entries()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        VobFile::parse(include_bytes!("../fixtures/tiny.sub"))
            .unwrap()
            .get_subtitle_entries()
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn srt_can_be_created_from_scratch() {
    let file = SrtFile::create(vec![(span(1500, 3700), "line1".to_string())]).unwrap();
    assert_eq!(
        String::from_utf8(file.to_data().unwrap()).unwrap(),
        "1\n00:00:01,500 --> 00:00:03,700\nline1\n\n"
    );
}

#[test]
fn charset_detection_and_bom_handling() {
    // GBK is guessed from the byte statistics
    let gbk = b"1\n00:00:10,000 --> 00:00:12,000\n\xc4\xe3\xba\xc3\xca\xc0\xbd\xe7\n\n";
    let entries = parse_bytes(SubtitleFormat::SubRip, gbk, None, 30.0)
        .unwrap()
        .get_subtitle_entries()
        .unwrap();
    assert_eq!(entries[0].line.as_deref(), Some("你好世界"));

    // a UTF-8 BOM does not end up in the text, and is not written back out
    let bom = "\u{feff}1\r\n00:00:10,000 --> 00:00:12,000\r\nBOM line\r\n\r\n";
    let file = parse_bytes(SubtitleFormat::SubRip, bom.as_bytes(), None, 30.0).unwrap();
    assert_eq!(
        file.get_subtitle_entries().unwrap()[0].line.as_deref(),
        Some("BOM line")
    );
    assert_eq!(
        String::from_utf8(file.to_data().unwrap()).unwrap(),
        "1\n00:00:10,000 --> 00:00:12,000\nBOM line\n\n"
    );

    // an explicit encoding overrides detection
    let latin1 = b"1\n00:00:10,000 --> 00:00:12,000\nCaf\xe9 na\xefve\n\n";
    let entries = parse_bytes(SubtitleFormat::SubRip, latin1, Some(encoding_rs::WINDOWS_1252), 30.0)
        .unwrap()
        .get_subtitle_entries()
        .unwrap();
    assert_eq!(entries[0].line.as_deref(), Some("Café naïve"));
}

#[test]
fn errors_implement_std_error_and_chain() {
    let error = parse_bytes(SubtitleFormat::SubRip, b"not a subtitle at all\n", None, 30.0).unwrap_err();

    let mut chain = vec![error.to_string()];
    let mut source = std::error::Error::source(&error);
    while let Some(cause) = source {
        chain.push(cause.to_string());
        source = cause.source();
    }

    // `alass-cli` prints the head as `error:` and every source as `caused by:`, so this
    // has to stay four links deep to reproduce what `subparse 0.7.0` + `failure` showed
    // for the same input.
    assert_eq!(
        chain,
        [
            "parsing the subtitle data failed",
            "parse error at line `0`",
            "expected SubRip index line, found 'not a subtitle at all'",
            "invalid digit found in string",
        ]
    );
}
