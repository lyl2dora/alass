// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! `SubStation Alpha` (`.ssa`/`.ass`) parsing and writing.

use crate::errors::{Error, ParseError, Result as SubtitleParserResult, SsaError};
use crate::formats::common::{
    dedup_string_parts, get_lines_non_destructive, split_bom, ssa_dialogue, ssa_timepoint, trim_non_destructive,
};
use crate::timetypes::{TimePoint, TimeSpan};
use crate::{SubtitleEntry, SubtitleFileInterface};

type Result<T> = std::result::Result<T, SsaError>;

// ////////////////////////////////////////////////////////////////////////////////////////////////
// SSA field info

/// Which comma-separated field of a `Dialogue:` line holds what.
struct SsaFieldsInfo {
    start_field_idx: usize,
    end_field_idx: usize,
    text_field_idx: usize,
    num_fields: usize,
}

impl SsaFieldsInfo {
    /// Parses a format line like "Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text".
    fn new_from_fields_info_line(line_num: usize, s: &str) -> Result<SsaFieldsInfo> {
        let field_info = s
            .strip_prefix("Format:")
            .expect("the caller only passes `Format:` lines");
        let mut start_field_idx: Option<usize> = None;
        let mut end_field_idx: Option<usize> = None;
        let mut text_field_idx: Option<usize> = None;

        // filter "Start" and "End" and "Text"
        let split_iter = field_info.split(',');
        let num_fields = split_iter.clone().count();
        for (i, field_name) in split_iter.enumerate() {
            let (name, field_idx) = match field_name.trim() {
                "Start" => ("Start", &mut start_field_idx),
                "End" => ("End", &mut end_field_idx),
                "Text" => ("Text", &mut text_field_idx),
                _ => continue,
            };
            if field_idx.is_some() {
                return Err(SsaError::SsaDuplicateField { line_num, f: name });
            }
            *field_idx = Some(i);
        }

        let text_field_idx = text_field_idx.ok_or(SsaError::SsaMissingField { line_num, f: "Text" })?;
        if text_field_idx != num_fields - 1 {
            return Err(SsaError::SsaTextFieldNotLast { line_num });
        }

        Ok(SsaFieldsInfo {
            start_field_idx: start_field_idx.ok_or(SsaError::SsaMissingField { line_num, f: "Start" })?,
            end_field_idx: end_field_idx.ok_or(SsaError::SsaMissingField { line_num, f: "End" })?,
            text_field_idx,
            num_fields,
        })
    }
}

// ////////////////////////////////////////////////////////////////////////////////////////////////
// SSA parser

impl SsaFile {
    /// Parse a `.ssa` subtitle string to `SsaFile`.
    pub fn parse(s: &str) -> SubtitleParserResult<SsaFile> {
        Self::parse_inner(s).map_err(|e| Error::from(ParseError::from(e)))
    }

    /// Parses a whole `.ssa` file from string.
    fn parse_inner(i: &str) -> Result<SsaFile> {
        let mut file_parts = Vec::new();
        let (bom, s) = split_bom(i);
        file_parts.push(SsaFilePart::Filler(bom.to_string()));

        // first we need to find and parse the format line, which then dictates how to parse the file
        let (line_num, field_info_line) = Self::get_format_info(s)?;
        let fields_info = SsaFieldsInfo::new_from_fields_info_line(line_num, field_info_line)?;

        // parse the dialog lines with the given format
        file_parts.append(&mut Self::parse_dialog_lines(&fields_info, s)?);
        Ok(SsaFile::new(file_parts))
    }

    /// Searches and parses a format line like "Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text".
    fn get_format_info(s: &str) -> Result<(usize, &str)> {
        let mut section_opt = None;
        for (line_num, line) in s.lines().enumerate() {
            // parse section headers like `[Events]`
            let trimmed_line = line.trim();
            if trimmed_line.starts_with('[') && trimmed_line.ends_with(']') {
                section_opt = Some(&trimmed_line[1..trimmed_line.len() - 1]);
            }

            // most sections have a format line, but we only want the one for the subtitle events
            if section_opt != Some("Events") {
                continue;
            }
            if !trimmed_line.starts_with("Format:") {
                continue;
            }
            return Ok((line_num, trimmed_line));
        }

        Err(SsaError::SsaFieldsInfoNotFound)
    }

    /// Filters file for lines like this and parses them:
    ///
    /// ```text
    /// "Dialogue: 1,0:22:43.52,0:22:46.22,ED-Romaji,,0,0,0,,{\fad(150,150)\blur0.5\bord1}some text"
    /// ```
    fn parse_dialog_lines(fields_info: &SsaFieldsInfo, s: &str) -> Result<Vec<SsaFilePart>> {
        let mut result = Vec::new();
        let mut section_opt: Option<String> = None;

        for (line_num, (line, newl)) in get_lines_non_destructive(s).into_iter().enumerate() {
            let trimmed_line = line.trim();

            // parse section headers like `[Events]`
            if trimmed_line.starts_with('[') && trimmed_line.ends_with(']') {
                section_opt = Some(trimmed_line[1..trimmed_line.len() - 1].to_string());
                result.push(SsaFilePart::Filler(line));
                result.push(SsaFilePart::Filler(newl));
                continue;
            }

            if section_opt.as_deref() != Some("Events") || !trimmed_line.starts_with("Dialogue:") {
                result.push(SsaFilePart::Filler(line));
                result.push(SsaFilePart::Filler(newl));
                continue;
            }

            result.append(&mut Self::parse_dialog_line(line_num, line.as_str(), fields_info)?);
            result.push(SsaFilePart::Filler(newl));
        }

        Ok(result)
    }

    /// Parse lines like:
    ///
    /// ```text
    /// "Dialogue: 1,0:22:43.52,0:22:46.22,ED-Romaji,,0,0,0,,{\fad(150,150)\blur0.5\bord1}some text"
    /// ```
    fn parse_dialog_line(line_num: usize, line: &str, fields_info: &SsaFieldsInfo) -> Result<Vec<SsaFilePart>> {
        let (ws1, ws2, fields, text) =
            ssa_dialogue(line, fields_info.num_fields).ok_or_else(|| SsaError::SsaDialogLineParseError {
                line_num,
                msg: format!(
                    "expected `Dialogue:` followed by {} comma-separated fields",
                    fields_info.num_fields
                ),
            })?;

        let mut result: Vec<SsaFilePart> = Vec::with_capacity(4 * fields.len() + 4);
        result.push(SsaFilePart::Filler(ws1.to_string()));
        result.push(SsaFilePart::Filler("Dialogue:".to_string()));
        result.push(SsaFilePart::Filler(ws2.to_string()));
        result.append(&mut Self::parse_fields(line_num, fields_info, fields)?);
        result.push(SsaFilePart::Text(text.to_string()));
        Ok(result)
    }

    /// Parses an array of fields with the "fields info".
    ///
    /// The fields (comma separated information) as an array like
    /// `vec!["1", "0:22:43.52", "0:22:46.22", "ED-Romaji", "", "0", "0", "0", ""]`.
    fn parse_fields(line_num: usize, fields_info: &SsaFieldsInfo, v: Vec<(&str, char)>) -> Result<Vec<SsaFilePart>> {
        let mut result = Vec::with_capacity(4 * v.len());
        for (i, (field, sep_char)) in v.into_iter().enumerate() {
            let (begin, field, end) = trim_non_destructive(field);

            let part = if i == fields_info.start_field_idx {
                SsaFilePart::TimespanStart(Self::parse_timepoint(line_num, field)?)
            } else if i == fields_info.end_field_idx {
                SsaFilePart::TimespanEnd(Self::parse_timepoint(line_num, field)?)
            } else if i == fields_info.text_field_idx {
                SsaFilePart::Text(field.to_string())
            } else {
                SsaFilePart::Filler(field.to_string())
            };

            result.push(SsaFilePart::Filler(begin.to_string()));
            result.push(part);
            result.push(SsaFilePart::Filler(end.to_string()));
            result.push(SsaFilePart::Filler(sep_char.to_string()));
        }
        Ok(result)
    }

    /// Something like "0:19:41.99".
    fn parse_timepoint(line_num: usize, s: &str) -> Result<TimePoint> {
        let msecs = ssa_timepoint(s).ok_or_else(|| SsaError::SsaWrongTimepointFormat {
            line_num,
            string: s.to_string(),
        })?;
        Ok(TimePoint::from_msecs(msecs))
    }
}

// ////////////////////////////////////////////////////////////////////////////////////////////////
// SSA file parts

#[derive(Debug, Clone)]
enum SsaFilePart {
    /// Spaces, field information, comments, unimportant fields, ...
    Filler(String),

    /// Timespan start of a dialogue line.
    TimespanStart(TimePoint),

    /// Timespan end of a dialogue line.
    TimespanEnd(TimePoint),

    /// Dialog lines.
    Text(String),
}

// ////////////////////////////////////////////////////////////////////////////////////////////////
// SSA file

/// Represents a reconstructable `.ssa`/`.ass` file.
///
/// All unimportant information (for this project) is saved into `SsaFilePart::Filler(...)`, so
/// a timespan-altered file still has the same fields etc.
#[derive(Debug, Clone)]
pub struct SsaFile {
    v: Vec<SsaFilePart>,
}

impl SsaFile {
    fn new(v: Vec<SsaFilePart>) -> SsaFile {
        // cleans up multiple fillers after another
        let new_file_parts = dedup_string_parts(v, |part: &mut SsaFilePart| match part {
            SsaFilePart::Filler(text) => Some(text),
            _ => None,
        });

        SsaFile { v: new_file_parts }
    }

    /// This function filters out all start times and end times, and returns them ordered
    /// (="(start, end, dialog)") so they can be easily read or written to.
    ///
    /// TODO: implement a single version that takes both `&mut` and `&` (dependent on HKT).
    fn get_subtitle_entries_mut<'a>(&'a mut self) -> Vec<(&'a mut TimePoint, &'a mut TimePoint, &'a mut String)> {
        let mut startpoint_buffer: Option<&'a mut TimePoint> = None;
        let mut endpoint_buffer: Option<&'a mut TimePoint> = None;

        // the extra block satisfies the borrow checker
        let timings: Vec<_> = {
            let filter_map_closure =
                |part: &'a mut SsaFilePart| -> Option<(&'a mut TimePoint, &'a mut TimePoint, &'a mut String)> {
                    match *part {
                        SsaFilePart::TimespanStart(ref mut start) => {
                            assert_eq!(startpoint_buffer, None); // parser should have ensured that no two consecutive SSA start times exist
                            startpoint_buffer = Some(start);
                            None
                        }
                        SsaFilePart::TimespanEnd(ref mut end) => {
                            assert_eq!(endpoint_buffer, None); // parser should have ensured that no two consecutive SSA end times exist
                            endpoint_buffer = Some(end);
                            None
                        }
                        SsaFilePart::Text(ref mut text) => {
                            // reset the timepoint buffers
                            let snatched_startpoint_buffer = startpoint_buffer.take();
                            let snatched_endpoint_buffer = endpoint_buffer.take();

                            let start = snatched_startpoint_buffer
                                .expect("SSA parser should have ensured that every line has a startpoint");
                            let end = snatched_endpoint_buffer
                                .expect("SSA parser should have ensured that every line has a endpoint");

                            Some((start, end, text))
                        }
                        SsaFilePart::Filler(_) => None,
                    }
                };

            self.v.iter_mut().filter_map(filter_map_closure).collect()
        };

        // every timespan should now consist of a beginning and a end (this should be ensured by parser)
        assert_eq!(startpoint_buffer, None);
        assert_eq!(endpoint_buffer, None);

        timings
    }
}

impl SubtitleFileInterface for SsaFile {
    fn get_subtitle_entries(&self) -> SubtitleParserResult<Vec<SubtitleEntry>> {
        // it's unfortunate we have to clone the file before using
        // `get_subtitle_entries_mut()`, but otherwise we'd have to copy the
        // `get_subtitle_entries_mut()` and create a non-mut-reference version
        // of it (much code duplication); I think a `clone` in this
        // not-time-critical code is acceptable, and after HKT become
        // available, this can be solved much nicer.
        let mut new_file = self.clone();
        let timings = new_file
            .get_subtitle_entries_mut()
            .into_iter()
            .map(|(&mut start, &mut end, text)| SubtitleEntry::new(TimeSpan::new(start, end), text.clone()))
            .collect();

        Ok(timings)
    }

    fn update_subtitle_entries(&mut self, new_subtitle_entries: &[SubtitleEntry]) -> SubtitleParserResult<()> {
        let subtitle_entries = self.get_subtitle_entries_mut();
        // required by the specification of this function
        if subtitle_entries.len() != new_subtitle_entries.len() {
            return Err(Error::EntryCountMismatch {
                expected: subtitle_entries.len(),
                provided: new_subtitle_entries.len(),
            });
        }

        for ((start_ref, end_ref, text_ref), new_entry_ref) in subtitle_entries.into_iter().zip(new_subtitle_entries) {
            *start_ref = new_entry_ref.timespan.start;
            *end_ref = new_entry_ref.timespan.end;
            if let Some(text) = &new_entry_ref.line {
                text_ref.clone_from(text);
            }
        }

        Ok(())
    }

    fn to_data(&self) -> SubtitleParserResult<Vec<u8>> {
        // timing to string like "0:00:22.21"
        let fn_timing_to_string = |t: TimePoint| {
            let p = t.abs();
            format!(
                "{}{}:{:02}:{:02}.{:02}",
                if t.msecs() < 0 { "-" } else { "" },
                p.hours(),
                p.mins_comp(),
                p.secs_comp(),
                p.csecs_comp()
            )
        };

        let fn_file_part_to_string = |part: &SsaFilePart| match *part {
            SsaFilePart::Filler(ref t) | SsaFilePart::Text(ref t) => t.clone(),
            SsaFilePart::TimespanStart(start) => fn_timing_to_string(start),
            SsaFilePart::TimespanEnd(end) => fn_timing_to_string(end),
        };

        let result: String = self.v.iter().map(fn_file_part_to_string).collect();

        Ok(result.into_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::SsaFile;
    use crate::timetypes::TimeDelta;
    use crate::{SubtitleEntry, SubtitleFileInterface};

    const SSA: &str = "[Script Info]\n\
                       ScriptType: v4.00+\n\
                       \n\
                       [Events]\n\
                       Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\n\
                       Dialogue: 0,0:00:10.00,0:00:12.00,Default,,0,0,0,,Hello there\n\
                       Dialogue: 0,0:00:15.50,0:00:17.50,Default,,0,0,0,,Second\n";

    fn to_string(file: &SsaFile) -> String {
        String::from_utf8(file.to_data().unwrap()).unwrap()
    }

    #[test]
    fn parse_is_non_destructive() {
        let file = SsaFile::parse(SSA).unwrap();
        assert_eq!(to_string(&file), SSA);
    }

    /// `subparse 0.7.0` rewrote every non-`Dialogue:` line ending as `\n`, so a CRLF
    /// file came back with mixed line endings.
    #[test]
    fn crlf_line_endings_are_preserved() {
        let crlf = SSA.replace('\n', "\r\n");
        let file = SsaFile::parse(&crlf).unwrap();
        assert_eq!(to_string(&file), crlf);
    }

    /// ...and a file without a trailing newline did not gain one.
    #[test]
    fn a_missing_trailing_newline_is_not_invented() {
        let without = SSA.trim_end_matches('\n');
        let file = SsaFile::parse(without).unwrap();
        assert_eq!(to_string(&file), without);
    }

    #[test]
    fn bom_is_preserved() {
        let with_bom = format!("\u{feff}{SSA}");
        let file = SsaFile::parse(&with_bom).unwrap();
        assert_eq!(to_string(&file), with_bom);
    }

    #[test]
    fn entries_are_read_and_written() {
        let mut file = SsaFile::parse(SSA).unwrap();
        let entries = file.get_subtitle_entries().unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(
            (entries[0].timespan.start.msecs(), entries[0].timespan.end.msecs()),
            (10_000, 12_000)
        );
        assert_eq!(entries[1].line.as_deref(), Some("Second"));

        let shifted: Vec<SubtitleEntry> = entries
            .iter()
            .map(|e| SubtitleEntry::from(e.timespan + TimeDelta::from_msecs(1234)))
            .collect();
        file.update_subtitle_entries(&shifted).unwrap();
        assert_eq!(
            to_string(&file),
            SSA.replace("0:00:10.00,0:00:12.00", "0:00:11.23,0:00:13.23")
                .replace("0:00:15.50,0:00:17.50", "0:00:16.73,0:00:18.73")
        );
    }

    #[test]
    fn field_whitespace_is_preserved() {
        let padded = SSA.replace(",0:00:10.00,", " , 0:00:10.00 ,");
        let file = SsaFile::parse(&padded).unwrap();
        assert_eq!(to_string(&file), padded);
    }

    #[test]
    fn a_colon_is_accepted_before_the_hundredths() {
        let file = SsaFile::parse(&SSA.replace("0:00:10.00", "0:00:10:00")).unwrap();
        assert_eq!(file.get_subtitle_entries().unwrap()[0].timespan.start.msecs(), 10_000);
    }

    #[test]
    fn broken_files_are_rejected() {
        // no `Format:` line inside `[Events]`
        assert!(SsaFile::parse("[Script Info]\nFormat: Start, End, Text\n").is_err());
        // `Text` is not last
        assert!(SsaFile::parse("[Events]\nFormat: Start, Text, End\n").is_err());
        // missing `Start`
        assert!(SsaFile::parse("[Events]\nFormat: Layer, End, Text\n").is_err());
        // a duplicated field
        assert!(SsaFile::parse("[Events]\nFormat: Start, Start, End, Text\n").is_err());
    }

    /// The wrong timepoint has to appear in the message; `subparse 0.7.0` printed a
    /// rendered parser error there instead.
    #[test]
    fn a_broken_timepoint_names_itself() {
        let error = SsaFile::parse(&SSA.replace("0:00:10.00", "10 seconds")).unwrap_err();
        let mut chain = Vec::new();
        let mut source: Option<&dyn std::error::Error> = Some(&error);
        while let Some(e) = source {
            chain.push(e.to_string());
            source = e.source();
        }
        assert!(
            chain
                .iter()
                .any(|m| m == "the timepoint `10 seconds` in line 5 has wrong format"),
            "{chain:?}"
        );
    }

    /// `subparse 0.7.0` accepted an indented `Format:` line in its search loop but then
    /// asserted on the untrimmed line, so this panicked with
    /// `assertion failed: s.starts_with("Format:")`.
    #[test]
    fn an_indented_format_line_is_accepted() {
        let indented = SSA.replace("Format: Layer", "  Format: Layer");
        let file = SsaFile::parse(&indented).unwrap();
        assert_eq!(file.get_subtitle_entries().unwrap().len(), 2);
        assert_eq!(to_string(&file), indented);
    }

    /// `Text` is the last field, so every comma after it belongs to the text.
    #[test]
    fn the_text_field_keeps_its_commas() {
        let input = SSA.replace("Hello there", "a,b,c,d,e");
        let file = SsaFile::parse(&input).unwrap();
        assert_eq!(
            file.get_subtitle_entries().unwrap()[0].line.as_deref(),
            Some("a,b,c,d,e")
        );
        assert_eq!(to_string(&file), input);
    }

    /// The `Format:` line decides which column is which, in any order.
    #[test]
    fn the_format_line_may_put_the_columns_in_any_order() {
        let input = "[Events]\n\
                     Format: Start, End, Layer, Style, Name, MarginL, MarginR, MarginV, Effect, Text\n\
                     Dialogue: 0:00:10.00,0:00:12.00,0,Default,,0,0,0,,Hello\n";
        let file = SsaFile::parse(input).unwrap();
        let entries = file.get_subtitle_entries().unwrap();
        assert_eq!(
            (entries[0].timespan.start.msecs(), entries[0].timespan.end.msecs()),
            (10_000, 12_000)
        );
        assert_eq!(entries[0].line.as_deref(), Some("Hello"));
        assert_eq!(to_string(&file), input);

        // three columns is enough
        let minimal = "[Events]\nFormat: Start, End, Text\nDialogue: 0:00:10.00,0:00:12.00,Hello\n";
        assert_eq!(
            SsaFile::parse(minimal).unwrap().get_subtitle_entries().unwrap().len(),
            1
        );
    }

    #[test]
    fn a_dialogue_line_with_too_few_fields_is_rejected() {
        assert!(SsaFile::parse(&SSA.replace(",Default,,0,0,0,,Hello there", "")).is_err());
    }
}
