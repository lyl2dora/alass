// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! `SubRip` (`.srt`) parsing and writing.

use crate::errors::{Error, Result as SubtitleParserResult, SubRipError, SubRipErrorKind};
use crate::formats::common::{split_bom, srt_timespan};
use crate::timetypes::{TimePoint, TimeSpan};
use crate::{SubtitleEntry, SubtitleFileInterface};

use std::iter::once;

type Result<T> = std::result::Result<T, SubRipError>;

/// The parsing works as a finite state machine. These are the states in it.
enum SrtParserState {
    /// An empty line or an index follows.
    Emptyline,

    /// A timing line follows.
    Index(i64),

    /// A dialog line or an empty line follows.
    Timing(i64, TimeSpan),

    /// A dialog line or an empty line follows; some dialog was already read.
    Dialog(i64, TimeSpan, Vec<String>),
}

#[derive(Debug, Clone)]
/// Represents a `.srt` file.
pub struct SrtFile {
    v: Vec<SrtLine>,
}

#[derive(Debug, Clone)]
/// A complete description of one `SubRip` subtitle line.
struct SrtLine {
    /// Start and end time of the subtitle.
    timespan: TimeSpan,

    /// Index/number of the line.
    index: i64,

    /// The dialog/text lines of the `SrtLine`.
    texts: Vec<String>,
}

impl SrtFile {
    /// Parse a `.srt` subtitle string to `SrtFile`.
    pub fn parse(s: &str) -> SubtitleParserResult<SrtFile> {
        Self::parse_file(s).map_err(|e| Error::from(crate::errors::ParseError::from(e)))
    }

    fn parse_file(i: &str) -> Result<SrtFile> {
        use self::SrtParserState::{Dialog, Emptyline, Index, Timing};

        let mut result: Vec<SrtLine> = Vec::new();

        // remove utf-8 bom
        let (_, s) = split_bom(i);

        let mut state: SrtParserState = Emptyline; // expect emptyline or index

        // the `once("")` is there so no last entry gets ignored
        for (line_num, line) in s.lines().chain(once("")).enumerate() {
            state = match state {
                Emptyline => {
                    if line.trim().is_empty() {
                        Emptyline
                    } else {
                        Index(Self::parse_index_line(line_num, line)?)
                    }
                }
                Index(index) => Timing(index, Self::parse_timespan_line(line_num, line)?),
                Timing(index, timespan) => Self::state_expect_dialog(line, &mut result, index, timespan, Vec::new()),
                Dialog(index, timespan, texts) => Self::state_expect_dialog(line, &mut result, index, timespan, texts),
            };
        }

        Ok(SrtFile { v: result })
    }

    fn state_expect_dialog(
        line: &str,
        result: &mut Vec<SrtLine>,
        index: i64,
        timespan: TimeSpan,
        mut texts: Vec<String>,
    ) -> SrtParserState {
        if line.trim().is_empty() {
            result.push(SrtLine { index, timespan, texts });
            SrtParserState::Emptyline
        } else {
            texts.push(line.trim().to_string());
            SrtParserState::Dialog(index, timespan, texts)
        }
    }

    /// Matches a line with a single index.
    fn parse_index_line(line_num: usize, s: &str) -> Result<i64> {
        s.trim().parse::<i64>().map_err(|cause| SubRipError {
            line_num,
            kind: SubRipErrorKind::ExpectedIndexLine {
                line: s.to_string(),
                cause,
            },
        })
    }

    /// Matches a `SubRip` timespan like `00:24:45,670 --> 00:24:45,680`.
    fn parse_timespan_line(line_num: usize, line: &str) -> Result<TimeSpan> {
        let (start, end) = srt_timespan(line).ok_or_else(|| SubRipError {
            line_num,
            kind: SubRipErrorKind::ExpectedTimestampLine { line: line.to_string() },
        })?;
        Ok(TimeSpan::new(TimePoint::from_msecs(start), TimePoint::from_msecs(end)))
    }
}

impl SubtitleFileInterface for SrtFile {
    fn get_subtitle_entries(&self) -> SubtitleParserResult<Vec<SubtitleEntry>> {
        let timings = self
            .v
            .iter()
            .map(|line| SubtitleEntry::new(line.timespan, line.texts.join("\n")))
            .collect();

        Ok(timings)
    }

    fn update_subtitle_entries(&mut self, new_subtitle_entries: &[SubtitleEntry]) -> SubtitleParserResult<()> {
        // required by the specification of this function
        if self.v.len() != new_subtitle_entries.len() {
            return Err(Error::EntryCountMismatch {
                expected: self.v.len(),
                provided: new_subtitle_entries.len(),
            });
        }

        for (line_ref, new_entry_ref) in self.v.iter_mut().zip(new_subtitle_entries) {
            line_ref.timespan = new_entry_ref.timespan;
            if let Some(text) = &new_entry_ref.line {
                line_ref.texts = text.lines().map(str::to_string).collect();
            }
        }

        Ok(())
    }

    fn to_data(&self) -> SubtitleParserResult<Vec<u8>> {
        let timepoint_to_str = |t: TimePoint| -> String {
            format!(
                "{:02}:{:02}:{:02},{:03}",
                t.hours(),
                t.mins_comp(),
                t.secs_comp(),
                t.msecs_comp()
            )
        };
        let line_to_str = |line: &SrtLine| -> String {
            format!(
                "{}\n{} --> {}\n{}\n\n",
                line.index,
                timepoint_to_str(line.timespan.start),
                timepoint_to_str(line.timespan.end),
                line.texts.join("\n")
            )
        };

        Ok(self.v.iter().map(line_to_str).collect::<String>().into_bytes())
    }
}

impl SrtFile {
    /// Creates a `.srt` file from scratch.
    pub fn create(v: Vec<(TimeSpan, String)>) -> SubtitleParserResult<SrtFile> {
        let file_parts = v
            .into_iter()
            .enumerate()
            .map(|(i, (ts, text))| SrtLine {
                index: i as i64 + 1,
                timespan: ts,
                texts: text.lines().map(str::to_string).collect(),
            })
            .collect();

        Ok(SrtFile { v: file_parts })
    }
}

#[cfg(test)]
mod tests {
    use super::SrtFile;
    use crate::SubtitleFileInterface;
    use crate::timetypes::{TimeDelta, TimePoint, TimeSpan};

    fn to_string(file: &SrtFile) -> String {
        String::from_utf8(file.to_data().unwrap()).unwrap()
    }

    #[test]
    fn create_srt_test() {
        let lines = vec![
            (
                TimeSpan::new(TimePoint::from_msecs(1500), TimePoint::from_msecs(3700)),
                "line1".to_string(),
            ),
            (
                TimeSpan::new(TimePoint::from_msecs(4500), TimePoint::from_msecs(8700)),
                "line2".to_string(),
            ),
        ];
        let file = SrtFile::create(lines).unwrap();

        let expected = "1\n00:00:01,500 --> 00:00:03,700\nline1\n\n2\n00:00:04,500 --> 00:00:08,700\nline2\n\n";
        assert_eq!(to_string(&file), expected);
    }

    #[test]
    fn parse_write_roundtrip() {
        let input = "1\n00:00:01,500 --> 00:00:03,700\nline1\n\n2\n00:00:04,500 --> 00:00:08,700\nline2a\nline2b\n\n";
        let file = SrtFile::parse(input).unwrap();
        assert_eq!(to_string(&file), input);

        let entries = file.get_subtitle_entries().unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].timespan.start.msecs(), 1500);
        assert_eq!(entries[1].line.as_deref(), Some("line2a\nline2b"));
    }

    #[test]
    fn crlf_and_bom_are_accepted() {
        let input = "\u{feff}1\r\n00:00:01,500 --> 00:00:03,700\r\nline1\r\n\r\n";
        let file = SrtFile::parse(input).unwrap();
        let entries = file.get_subtitle_entries().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].line.as_deref(), Some("line1"));
        // the writer always emits LF and never a BOM
        assert_eq!(to_string(&file), "1\n00:00:01,500 --> 00:00:03,700\nline1\n\n");
    }

    #[test]
    fn shift_and_write() {
        let input = "1\n00:00:01,500 --> 00:00:03,700\nline1\n\n";
        let mut file = SrtFile::parse(input).unwrap();
        let shifted: Vec<_> = file
            .get_subtitle_entries()
            .unwrap()
            .into_iter()
            .map(|e| crate::SubtitleEntry::from(e.timespan + TimeDelta::from_msecs(1234)))
            .collect();
        file.update_subtitle_entries(&shifted).unwrap();
        // `line == None` must leave the text alone
        assert_eq!(to_string(&file), "1\n00:00:02,734 --> 00:00:04,934\nline1\n\n");
    }

    #[test]
    fn wrong_entry_count_is_an_error() {
        let mut file = SrtFile::parse("1\n00:00:01,500 --> 00:00:03,700\nline1\n\n").unwrap();
        assert!(file.update_subtitle_entries(&[]).is_err());
    }

    #[test]
    fn broken_files_are_rejected() {
        assert_eq!(
            SrtFile::parse("x\n00:00:01,500 --> 00:00:03,700\nline1\n\n")
                .unwrap_err()
                .to_string(),
            "parsing the subtitle data failed"
        );
        assert!(SrtFile::parse("1\nnot a timespan\nline1\n\n").is_err());
        // a 20-digit component used to panic; now it is a parse error
        assert!(SrtFile::parse("1\n99999999999999999999:00:01,500 --> 00:00:03,700\nl\n\n").is_err());
    }

    /// The components are plain numbers, not fixed-width fields - checked against
    /// `subparse 0.7.0`, which accepted the same three spellings.
    #[test]
    fn timestamp_components_may_have_any_number_of_digits() {
        for (input, start, end) in [
            ("0:0:1,5", 1005, 2007),
            ("00:00:01,50", 1050, 2070),
            ("0000:0000:0001,0500", 1500, 2700),
        ] {
            let second = match input {
                "0:0:1,5" => "0:0:2,7",
                "00:00:01,50" => "00:00:02,70",
                _ => "0000:0000:0002,0700",
            };
            let file = SrtFile::parse(&format!("1\n{input} --> {second}\nHello\n\n")).unwrap();
            let entries = file.get_subtitle_entries().unwrap();
            assert_eq!(
                (entries[0].timespan.start.msecs(), entries[0].timespan.end.msecs()),
                (start, end),
                "{input}"
            );
        }
    }

    /// `-00:00:10,000` is `from_components(-0, 0, 10, 0)`, and `-0` is `0`: the sign is
    /// swallowed. `subparse 0.7.0` did exactly the same, so it is pinned rather than
    /// "fixed".
    #[test]
    fn a_minus_on_the_hour_component_alone_does_not_make_the_time_negative() {
        let file = SrtFile::parse("1\n-00:00:10,000 --> 00:00:12,000\nHello\n\n").unwrap();
        assert_eq!(file.get_subtitle_entries().unwrap()[0].timespan.start.msecs(), 10_000);
    }

    #[test]
    fn text_may_contain_an_arrow() {
        let file = SrtFile::parse("1\n00:00:10,000 --> 00:00:12,000\na --> b\n\n").unwrap();
        assert_eq!(file.get_subtitle_entries().unwrap()[0].line.as_deref(), Some("a --> b"));
    }

    #[test]
    fn a_last_entry_without_a_trailing_blank_line_is_not_lost() {
        let file = SrtFile::parse("1\n00:00:10,000 --> 00:00:12,000\nHello").unwrap();
        assert_eq!(file.get_subtitle_entries().unwrap().len(), 1);
        assert_eq!(to_string(&file), "1\n00:00:10,000 --> 00:00:12,000\nHello\n\n");
    }

    /// Entries are neither sorted nor merged; out-of-order and overlapping input comes
    /// back in file order, indices and all.
    #[test]
    fn out_of_order_and_overlapping_entries_are_left_alone() {
        let input = "2\n00:00:15,000 --> 00:00:17,000\nB\n\n1\n00:00:10,000 --> 00:00:12,000\nA\n\n";
        let file = SrtFile::parse(input).unwrap();
        let entries = file.get_subtitle_entries().unwrap();
        assert_eq!(
            entries.iter().map(|e| e.timespan.start.msecs()).collect::<Vec<_>>(),
            [15_000, 10_000]
        );
        assert_eq!(to_string(&file), input);

        let overlapping = "1\n00:00:10,000 --> 00:00:20,000\nA\n\n2\n00:00:12,000 --> 00:00:14,000\nB\n\n";
        assert_eq!(to_string(&SrtFile::parse(overlapping).unwrap()), overlapping);
    }

    #[test]
    fn empty_input_parses_to_no_entries() {
        let file = SrtFile::parse("").unwrap();
        assert_eq!(file.get_subtitle_entries().unwrap().len(), 0);
        assert_eq!(to_string(&file), "");
    }
}
