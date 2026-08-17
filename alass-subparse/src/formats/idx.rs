// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! `VobSub` index (`.idx`) parsing and writing.

use crate::errors::{Error, IdxError, ParseError, Result as SubtitleParserResult};
use crate::formats::common::{dedup_string_parts, get_lines_non_destructive, idx_line, idx_timestamp, split_bom};
use crate::timetypes::{TimeDelta, TimePoint, TimeSpan};
use crate::{SubtitleEntry, SubtitleFileInterface};

use std::iter::once;

type Result<T> = std::result::Result<T, IdxError>;

// ////////////////////////////////////////////////////////////////////////////////////////////////
// .idx file parts

#[derive(Debug, Clone)]
enum IdxFilePart {
    /// Spaces, field information, comments, unimportant fields, ...
    Filler(String),

    /// Represents a parsed time string like "00:42:20:204".
    Timestamp(TimePoint),
}

// ////////////////////////////////////////////////////////////////////////////////////////////////
// .idx file

/// Represents a reconstructable `.idx` file.
///
/// All (for this project) unimportant information is saved into `IdxFilePart::Filler(...)`, so
/// a timespan-altered file still has the same meta-information.
#[derive(Debug, Clone)]
pub struct IdxFile {
    v: Vec<IdxFilePart>,
}

impl IdxFile {
    fn new(v: Vec<IdxFilePart>) -> IdxFile {
        // cleans up multiple fillers after another
        let new_file_parts = dedup_string_parts(v, |part: &mut IdxFilePart| match part {
            IdxFilePart::Filler(text) => Some(text),
            IdxFilePart::Timestamp(_) => None,
        });
        IdxFile { v: new_file_parts }
    }

    /// The number of timestamps in the file, which is the number of subtitle entries.
    fn timestamp_count(&self) -> usize {
        self.v
            .iter()
            .filter(|part| matches!(part, IdxFilePart::Timestamp(_)))
            .count()
    }
}

impl SubtitleFileInterface for IdxFile {
    fn get_subtitle_entries(&self) -> SubtitleParserResult<Vec<SubtitleEntry>> {
        let timings: Vec<_> = self
            .v
            .iter()
            .filter_map(|file_part| match *file_part {
                IdxFilePart::Filler(_) => None,
                IdxFilePart::Timestamp(t) => Some(t),
            })
            .collect();

        Ok(match timings.last() {
            Some(&last_timing) => {
                // .idx files do not store timespans. Every subtitle is shown until the next subtitle
                // starts. Mpv shows the last subtitle for exactly one minute.
                let next_timings = timings
                    .iter()
                    .copied()
                    .skip(1)
                    .chain(once(last_timing + TimeDelta::from_mins(1)));
                timings
                    .iter()
                    .copied()
                    .zip(next_timings)
                    .map(|(start, end)| TimeSpan::new(start, end))
                    .map(SubtitleEntry::from)
                    .collect()
            }
            None => {
                // no timings
                Vec::new()
            }
        })
    }

    fn update_subtitle_entries(&mut self, ts: &[SubtitleEntry]) -> SubtitleParserResult<()> {
        // required by the specification of this function
        let expected = self.timestamp_count();
        if expected != ts.len() {
            return Err(Error::EntryCountMismatch {
                expected,
                provided: ts.len(),
            });
        }

        let mut count = 0;
        for file_part_ref in &mut self.v {
            match file_part_ref {
                IdxFilePart::Filler(_) => {}
                IdxFilePart::Timestamp(this_ts_ref) => {
                    *this_ts_ref = ts[count].timespan.start;
                    count += 1;
                }
            }
        }

        Ok(())
    }

    fn to_data(&self) -> SubtitleParserResult<Vec<u8>> {
        // timing to string like "00:03:28:308"
        let fn_timing_to_string = |t: TimePoint| {
            let p = t.abs();
            format!(
                "{}{:02}:{:02}:{:02}:{:03}",
                if t.msecs() < 0 { "-" } else { "" },
                p.hours(),
                p.mins_comp(),
                p.secs_comp(),
                p.msecs_comp()
            )
        };

        let fn_file_part_to_string = |part: &IdxFilePart| match *part {
            IdxFilePart::Filler(ref t) => t.clone(),
            IdxFilePart::Timestamp(t) => fn_timing_to_string(t),
        };

        let result: String = self.v.iter().map(fn_file_part_to_string).collect();

        Ok(result.into_bytes())
    }
}

// ////////////////////////////////////////////////////////////////////////////////////////////////
// .idx parser

impl IdxFile {
    /// Parse a `.idx` subtitle string to `IdxFile`.
    pub fn parse(s: &str) -> SubtitleParserResult<IdxFile> {
        Self::parse_inner(s).map_err(|e| Error::from(ParseError::from(e)))
    }

    fn parse_inner(i: &str) -> Result<IdxFile> {
        // remove utf-8 BOM
        let mut result = Vec::new();
        let (bom, s) = split_bom(i);
        result.push(IdxFilePart::Filler(bom.to_string()));

        for (line_num, (line, newl)) in get_lines_non_destructive(s).into_iter().enumerate() {
            result.append(&mut Self::parse_line(line_num, line)?);
            result.push(IdxFilePart::Filler(newl));
        }

        Ok(IdxFile::new(result))
    }

    fn parse_line(line_num: usize, s: String) -> Result<Vec<IdxFilePart>> {
        if !s.trim_start().starts_with("timestamp:") {
            return Ok(vec![IdxFilePart::Filler(s)]);
        }

        let (ws1, keyword, ws2, timestamp_str, rest) = idx_line(&s).ok_or_else(|| IdxError::IdxLineParseError {
            line_num,
            msg: "expected a line starting with `timestamp:`".to_string(),
        })?;

        Ok(vec![
            IdxFilePart::Filler(ws1.to_string()),
            IdxFilePart::Filler(keyword.to_string()),
            IdxFilePart::Filler(ws2.to_string()),
            IdxFilePart::Timestamp(Self::parse_timestamp(line_num, timestamp_str)?),
            IdxFilePart::Filler(rest.to_string()),
        ])
    }

    /// Parse an .idx timestamp like `00:41:36:961`.
    fn parse_timestamp(line_num: usize, s: &str) -> Result<TimePoint> {
        let msecs = idx_timestamp(s).ok_or_else(|| IdxError::IdxLineParseError {
            line_num,
            msg: format!("expected a timestamp of the form `00:41:36:961`, found `{s}`"),
        })?;
        Ok(TimePoint::from_msecs(msecs))
    }
}

#[cfg(test)]
mod tests {
    use super::IdxFile;
    use crate::timetypes::TimeDelta;
    use crate::{SubtitleEntry, SubtitleFileInterface};

    const IDX: &str = "# VobSub index file, v7 (do not modify this line!)\n\
                       id: en, index: 0\n\
                       timestamp: 00:00:10:000, filepos: 000000000\n\
                       timestamp: 00:00:15:500, filepos: 000001000\n";

    fn to_string(file: &IdxFile) -> String {
        String::from_utf8(file.to_data().unwrap()).unwrap()
    }

    #[test]
    fn parse_is_non_destructive() {
        let file = IdxFile::parse(IDX).unwrap();
        assert_eq!(to_string(&file), IDX);
    }

    #[test]
    fn entries_span_until_the_next_timestamp() {
        let entries = IdxFile::parse(IDX).unwrap().get_subtitle_entries().unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(
            (entries[0].timespan.start.msecs(), entries[0].timespan.end.msecs()),
            (10_000, 15_500)
        );
        // the last entry is shown for one minute
        assert_eq!(
            (entries[1].timespan.start.msecs(), entries[1].timespan.end.msecs()),
            (15_500, 75_500)
        );
        assert!(entries.iter().all(|e| e.line.is_none()));
    }

    /// Regression test: `subparse 0.7.0` indexed with `count - 1` starting at
    /// `count == 0`, so this panicked with an index underflow on every `.idx` file.
    #[test]
    fn update_subtitle_entries_shifts_every_timestamp() {
        let mut file = IdxFile::parse(IDX).unwrap();
        let shifted: Vec<SubtitleEntry> = file
            .get_subtitle_entries()
            .unwrap()
            .into_iter()
            .map(|e| SubtitleEntry::from(e.timespan + TimeDelta::from_msecs(1234)))
            .collect();
        file.update_subtitle_entries(&shifted).unwrap();

        assert_eq!(
            to_string(&file),
            IDX.replace("00:00:10:000", "00:00:11:234")
                .replace("00:00:15:500", "00:00:16:734")
        );
    }

    #[test]
    fn wrong_entry_count_is_an_error_not_a_panic() {
        let mut file = IdxFile::parse(IDX).unwrap();
        assert!(file.update_subtitle_entries(&[]).is_err());
    }

    #[test]
    fn negative_timestamps_round_trip() {
        let mut file = IdxFile::parse(IDX).unwrap();
        let shifted: Vec<SubtitleEntry> = file
            .get_subtitle_entries()
            .unwrap()
            .into_iter()
            .map(|e| SubtitleEntry::from(e.timespan - TimeDelta::from_secs(11)))
            .collect();
        file.update_subtitle_entries(&shifted).unwrap();
        assert!(to_string(&file).contains("timestamp: -00:00:01:000"));
    }

    #[test]
    fn crlf_line_endings_are_preserved() {
        let crlf = IDX.replace('\n', "\r\n");
        let file = IdxFile::parse(&crlf).unwrap();
        assert_eq!(to_string(&file), crlf);
    }

    #[test]
    fn broken_timestamps_are_rejected() {
        assert!(IdxFile::parse("timestamp: 00:00:10.000, filepos: 0\n").is_err());
        assert!(IdxFile::parse("timestamp: nope\n").is_err());
    }

    /// A `timestamp:` that is not at the start of the line (after spaces and tabs) is
    /// just text, so a commented-out one is left alone.
    #[test]
    fn a_timestamp_inside_a_comment_is_not_a_timestamp() {
        let input = "# timestamp: 00:00:10:000\ntimestamp: 00:00:20:000, filepos: 0\n";
        let file = IdxFile::parse(input).unwrap();
        let entries = file.get_subtitle_entries().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].timespan.start.msecs(), 20_000);
        assert_eq!(to_string(&file), input);
    }

    #[test]
    fn a_file_without_timestamps_has_no_entries() {
        let file = IdxFile::parse("# nothing here\n").unwrap();
        assert_eq!(file.get_subtitle_entries().unwrap().len(), 0);
    }
}
