// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! `MicroDVD` (`.sub`) parsing and writing.

use crate::errors::{Error, MicroDvdError, MicroDvdErrorKind, ParseError, Result as SubtitleParserResult};
use crate::formats::common::{MdvdSubLine, mdvd_line, split_bom};
use crate::timetypes::{TimePoint, TimeSpan};
use crate::{SubtitleEntry, SubtitleFileInterface};

use std::collections::BTreeSet;
use std::fmt::Write as _;

type Result<T> = std::result::Result<T, MicroDvdError>;

/// Represents a formatting like "{y:i}" (display text in italics).
///
/// TODO: `MdvdFormatting` is a stub for the future where this enum holds specialized variants for different options.
#[derive(Debug, Clone, Hash, PartialEq, Eq, PartialOrd, Ord)]
enum MdvdFormatting {
    /// A format option that is not directly supported.
    Unknown(String),
}

impl From<&str> for MdvdFormatting {
    fn from(f: &str) -> MdvdFormatting {
        MdvdFormatting::Unknown(Self::lowercase_first_char(f))
    }
}

impl MdvdFormatting {
    /// Is this a single line formatting (e.g. `y:i`) or a multi-line formatting (e.g `Y:i`)?
    fn is_container_line_formatting(f: &str) -> bool {
        f.chars().next().is_some_and(char::is_uppercase)
    }

    /// Applies `to_lowercase()` to the first char, leaves the rest of the characters untouched.
    fn lowercase_first_char(s: &str) -> String {
        let mut c = s.chars();
        match c.next() {
            None => String::new(),
            Some(f) => f.to_lowercase().collect::<String>() + c.as_str(),
        }
    }

    /// Applies `to_uppercase()` to the first char, leaves the rest of the characters untouched.
    fn uppercase_first_char(s: &str) -> String {
        let mut c = s.chars();
        match c.next() {
            None => String::new(),
            Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        }
    }

    /// Convert a `MdvdFormatting` to a string which can be used in `.sub` files.
    fn to_formatting_string(&self, multiline: bool) -> String {
        let MdvdFormatting::Unknown(s) = self;
        if multiline {
            Self::uppercase_first_char(s)
        } else {
            Self::lowercase_first_char(s)
        }
    }
}

#[derive(Debug, Clone)]
/// Represents a reconstructable `.sub`(`MicroDVD`) file.
pub struct MdvdFile {
    /// Number of frames per second of the associated video (default 25)
    /// -> start/end frames can be converted to timestamps
    fps: f64,

    /// all lines and multilines
    v: Vec<MdvdLine>,
}

/// Holds the description of a single line.
#[derive(Debug, Clone)]
struct MdvdLine {
    /// The start frame.
    start_frame: i64,

    /// The end frame.
    end_frame: i64,

    /// Formatting that affects all contained single lines.
    formatting: Vec<MdvdFormatting>,

    /// The (dialog) text of the line.
    text: String,
}

impl MdvdLine {
    fn to_subtitle_entry(&self, fps: f64) -> SubtitleEntry {
        SubtitleEntry {
            timespan: TimeSpan::new(
                TimePoint::from_msecs((self.start_frame as f64 * 1000.0 / fps) as i64),
                TimePoint::from_msecs((self.end_frame as f64 * 1000.0 / fps) as i64),
            ),
            line: Some(self.text.clone()),
        }
    }
}

impl MdvdFile {
    /// Parse a `MicroDVD` `.sub` subtitle string to `MdvdFile`.
    pub fn parse(s: &str, fps: f64) -> SubtitleParserResult<MdvdFile> {
        Self::parse_file(s, fps).map_err(|e| Error::from(ParseError::from(e)))
    }

    fn parse_file(i: &str, fps: f64) -> Result<MdvdFile> {
        let mut result: Vec<MdvdLine> = Vec::new();

        // remove utf-8 bom
        let (_, s) = split_bom(i);

        for (line_num, line) in s.lines().enumerate() {
            // a line looks like "{0}{25}{c:$0000ff}{y:b,u}{f:DeJaVuSans}{s:12}Hello!|{y:i}Hello2!" where
            // 0 and 25 are the start and end frames and the other information is the formatting.
            result.append(&mut Self::parse_line(line_num, line)?);
        }

        Ok(MdvdFile { fps, v: result })
    }

    /// Parses something like "{0}{25}{C:$0000ff}{y:b,u}{f:DeJaVuSans}{s:12}Hello!|{s:15}Hello2!".
    fn parse_line(line_num: usize, line: &str) -> Result<Vec<MdvdLine>> {
        let (start_frame, end_frame, sub_lines) = mdvd_line(line).ok_or_else(|| MicroDvdError {
            line_num,
            kind: MicroDvdErrorKind::ExpectedSubtitleLine { line: line.to_string() },
        })?;

        Ok(Self::construct_mdvd_lines(start_frame, end_frame, sub_lines))
    }

    /// Construct (possibly multiple) `MdvdLines` from a deconstructed file line
    /// like "{C:$0000ff}{y:b,u}{f:DeJaVuSans}{s:12}Hello!|{s:15}Hello2!".
    ///
    /// The third parameter is for the example
    /// like `[(["C:$0000ff", "y:b,u", "f:DeJaVuSans", "s:12"], "Hello!"), (["s:15"], "Hello2!")]`.
    fn construct_mdvd_lines(
        start_frame: i64,
        end_frame: i64,
        fmt_strs_and_lines: Vec<MdvdSubLine<'_>>,
    ) -> Vec<MdvdLine> {
        // saves all multiline formatting
        let mut cline_fmts: Vec<MdvdFormatting> = Vec::new();

        // convert the formatting strings to `MdvdFormatting` objects and split between multi-line and single-line formatting
        let fmts_and_lines = fmt_strs_and_lines
            .into_iter()
            .map(|(fmts, text)| (Self::string_to_formatting(&mut cline_fmts, fmts), text))
            .collect::<Vec<_>>();

        // now we also have all multi-line formattings in `cline_fmts`

        // finish creation of `MdvdLine`s
        fmts_and_lines
            .into_iter()
            .map(|(sline_fmts, text)| MdvdLine {
                start_frame,
                end_frame,
                text: text.to_string(),
                formatting: cline_fmts.iter().cloned().chain(sline_fmts).collect(),
            })
            .collect()
    }

    /// Convert `MicroDVD` formatting strings to `MdvdFormatting` objects.
    ///
    /// Moves multiline formattings and single line formattings into different vectors.
    fn string_to_formatting(multiline_formatting: &mut Vec<MdvdFormatting>, fmts: Vec<&str>) -> Vec<MdvdFormatting> {
        // split multiline-formatting (e.g "Y:b") and single-line formatting (e.g "y:b")
        let (cline_fmts_str, sline_fmts_str): (Vec<_>, Vec<_>) = fmts
            .into_iter()
            .partition(|fmt_str| MdvdFormatting::is_container_line_formatting(fmt_str));

        multiline_formatting.extend(cline_fmts_str.into_iter().map(MdvdFormatting::from));
        sline_fmts_str.into_iter().map(MdvdFormatting::from).collect()
    }
}

impl SubtitleFileInterface for MdvdFile {
    fn get_subtitle_entries(&self) -> SubtitleParserResult<Vec<SubtitleEntry>> {
        Ok(self.v.iter().map(|line| line.to_subtitle_entry(self.fps)).collect())
    }

    fn update_subtitle_entries(&mut self, new_subtitle_entries: &[SubtitleEntry]) -> SubtitleParserResult<()> {
        // required by the specification of this function
        if self.v.len() != new_subtitle_entries.len() {
            return Err(Error::EntryCountMismatch {
                expected: self.v.len(),
                provided: new_subtitle_entries.len(),
            });
        }

        for (line, new_entry) in self.v.iter_mut().zip(new_subtitle_entries) {
            line.start_frame = (new_entry.timespan.start.secs_f64() * self.fps) as i64;
            line.end_frame = (new_entry.timespan.end.secs_f64() * self.fps) as i64;

            if let Some(text) = &new_entry.line {
                line.text.clone_from(text);
            }
        }

        Ok(())
    }

    fn to_data(&self) -> SubtitleParserResult<Vec<u8>> {
        let mut sorted_list = self.v.clone();
        sorted_list.sort_by_key(|line| (line.start_frame, line.end_frame));

        let mut result = String::new();

        // All single lines in a group have the same start and end time
        //  -> the .sub file format lets them be on the same line with "{0}{1000}Text1|Text2".
        for (gi, group) in group_by_frames(sorted_list).into_iter().enumerate() {
            if gi != 0 {
                result.push('\n');
            }

            let (start_frame, end_frame) = (group[0].start_frame, group[0].end_frame);
            let group_len = group.len();
            let (formattings, texts): (Vec<BTreeSet<MdvdFormatting>>, Vec<String>) = group
                .into_iter()
                .map(|line| (line.formatting.into_iter().collect(), line.text))
                .unzip();

            // find common formatting in all lines
            let common_formatting: BTreeSet<MdvdFormatting> = if group_len == 1 {
                // if this "group" only has a single line, let's say that every formatting is individual
                BTreeSet::new()
            } else {
                formattings.iter().skip(1).fold(formattings[0].clone(), |acc, set| {
                    acc.intersection(set).cloned().collect()
                })
            };

            let individual_formattings = formattings
                .into_iter()
                .map(|formatting| formatting.difference(&common_formatting).cloned().collect())
                .collect::<Vec<BTreeSet<MdvdFormatting>>>();

            // `write!` into a `String` cannot fail
            let _ = write!(result, "{{{start_frame}}}{{{end_frame}}}");

            for formatting in &common_formatting {
                let _ = write!(result, "{{{}}}", formatting.to_formatting_string(true));
            }

            for (i, (individual_formatting, text)) in individual_formattings.into_iter().zip(texts).enumerate() {
                if i != 0 {
                    result.push('|');
                }

                for formatting in &individual_formatting {
                    let _ = write!(result, "{{{}}}", formatting.to_formatting_string(false));
                }

                result.push_str(&text);
            }
        }

        Ok(result.into_bytes())
    }
}

/// Groups an already sorted list of lines by their start and end frame.
fn group_by_frames(sorted_list: Vec<MdvdLine>) -> Vec<Vec<MdvdLine>> {
    let mut groups: Vec<Vec<MdvdLine>> = Vec::new();
    for line in sorted_list {
        match groups.last_mut() {
            Some(group) if group[0].start_frame == line.start_frame && group[0].end_frame == line.end_frame => {
                group.push(line)
            }
            _ => groups.push(vec![line]),
        }
    }
    groups
}

#[cfg(test)]
mod tests {
    use super::MdvdFile;
    use crate::timetypes::TimeDelta;
    use crate::{SubtitleEntry, SubtitleFileInterface};

    /// Parse string with `MdvdFile`, and re-encode it with `MdvdFile`.
    fn mdvd_reconstruct(s: &str) -> String {
        let file = MdvdFile::parse(s, 25.0).unwrap();
        let data = file.to_data().unwrap();
        String::from_utf8(data).unwrap()
    }

    /// Parse and re-construct `MicroDVD` files and test them against expected output.
    fn test_mdvd(input: &str, expected: &str) {
        // if we put the `input` into the parser, we expect a specific (cleaned-up) output
        assert_eq!(mdvd_reconstruct(input), expected);

        // if we reconstruct the cleaned-up output, we expect that nothing changes
        assert_eq!(mdvd_reconstruct(expected), expected);
    }

    #[test]
    fn mdvd_test_reconstruction() {
        // simple examples
        test_mdvd("{0}{25}Hello!", "{0}{25}Hello!");
        test_mdvd("{0}{25}{y:i}Hello!", "{0}{25}{y:i}Hello!");
        test_mdvd("{0}{25}{Y:i}Hello!", "{0}{25}{y:i}Hello!");
        test_mdvd("{0}{25}{Y:i}\n", "{0}{25}{y:i}");

        // cleanup formattings in a file
        test_mdvd("{0}{25}{y:i}Text1|{y:i}Text2", "{0}{25}{Y:i}Text1|Text2");
        test_mdvd("{0}{25}{y:i}Text1\n{0}{25}{y:i}Text2", "{0}{25}{Y:i}Text1|Text2");
        test_mdvd(
            "{0}{25}{y:i}{y:b}Text1\n{0}{25}{y:i}Text2",
            "{0}{25}{Y:i}{y:b}Text1|Text2",
        );

        // these can't be condensed, because the lines have different times
        test_mdvd(
            "{0}{25}{y:i}Text1\n{0}{26}{y:i}Text2",
            "{0}{25}{y:i}Text1\n{0}{26}{y:i}Text2",
        );
    }

    /// `subparse 0.7.0` iterated over a `HashSet` here, so a line with two or more
    /// formattings serialized in a different order on every run.
    #[test]
    fn formatting_order_is_deterministic() {
        let input = "{0}{25}{y:i}{y:b}{y:u}Text1|{y:i}{y:b}{y:u}Text2";
        let first = mdvd_reconstruct(input);
        for _ in 0..16 {
            assert_eq!(mdvd_reconstruct(input), first);
        }
        assert_eq!(first, "{0}{25}{Y:b}{Y:i}{Y:u}Text1|Text2");
    }

    #[test]
    fn entries_and_shifting() {
        let mut file = MdvdFile::parse("{100}{200}Hello|World\n{250}{400}Second line\n", 30.0).unwrap();
        let entries = file.get_subtitle_entries().unwrap();
        assert_eq!(entries.len(), 3);
        assert_eq!(
            (entries[0].timespan.start.msecs(), entries[0].timespan.end.msecs()),
            (3333, 6666)
        );
        assert_eq!(entries[2].line.as_deref(), Some("Second line"));

        let shifted: Vec<SubtitleEntry> = entries
            .iter()
            .map(|e| SubtitleEntry::from(e.timespan + TimeDelta::from_msecs(1000)))
            .collect();
        file.update_subtitle_entries(&shifted).unwrap();
        assert_eq!(
            String::from_utf8(file.to_data().unwrap()).unwrap(),
            "{129}{229}Hello|World\n{279}{429}Second line"
        );
    }

    #[test]
    fn bom_is_ignored() {
        assert_eq!(mdvd_reconstruct("\u{feff}{0}{25}Hello!"), "{0}{25}Hello!");
    }

    #[test]
    fn crlf_lines_are_accepted() {
        assert_eq!(mdvd_reconstruct("{0}{25}A\r\n{30}{40}B\r\n"), "{0}{25}A\n{30}{40}B");
    }

    #[test]
    fn broken_lines_are_rejected() {
        assert!(MdvdFile::parse("no braces here", 25.0).is_err());
        assert!(MdvdFile::parse("{0}{25}{unclosed", 25.0).is_err());
        // a number that does not fit into an i64 used to panic
        assert!(MdvdFile::parse("{99999999999999999999}{25}x", 25.0).is_err());
    }

    /// `{}` is an empty formatting tag, not text: it is kept as a tag, and repeats of
    /// it collapse into one because formattings are a set. Both match `subparse 0.7.0`.
    #[test]
    fn empty_formatting_braces_are_tags_not_text() {
        for input in ["{0}{25}{}Hello", "{0}{25}{}{}Hello"] {
            let file = MdvdFile::parse(input, 25.0).unwrap();
            let entries = file.get_subtitle_entries().unwrap();
            assert_eq!(entries.len(), 1);
            assert_eq!(entries[0].line.as_deref(), Some("Hello"), "{input}");
            assert_eq!(mdvd_reconstruct(input), "{0}{25}{}Hello", "{input}");
        }
    }

    /// `|` splits sub-lines, so an empty part really is an empty sub-line.
    #[test]
    fn empty_pipe_separated_parts_become_empty_sub_lines() {
        let texts = |s: &str| {
            MdvdFile::parse(s, 25.0)
                .unwrap()
                .get_subtitle_entries()
                .unwrap()
                .iter()
                .map(|e| e.line.clone().unwrap())
                .collect::<Vec<_>>()
        };
        assert_eq!(texts("{0}{25}A||B"), ["A", "", "B"]);
        assert_eq!(texts("{0}{25}A|"), ["A", ""]);
        assert_eq!(texts("{0}{25}"), [""]);
    }

    #[test]
    fn wrong_entry_count_is_an_error() {
        let mut file = MdvdFile::parse("{0}{25}Hello", 25.0).unwrap();
        assert!(file.update_subtitle_entries(&[]).is_err());
    }
}
