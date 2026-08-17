// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! Small scanning helpers shared by the line-oriented text parsers.
//!
//! Every subtitle format handled by this crate is line-oriented and has a fixed
//! shape, so the parsers are plain `&str` scanners: each one either consumes the
//! exact syntax it expects and returns the remainder, or returns `None`.

/// A line together with the exact line ending that terminated it.
///
/// Used by `get_lines_non_destructive()` so a file can be rebuilt byte for byte.
type SplittedLine = (
    String, /* string */
    String, /* newline string like \n or \r\n */
);

/// Splits off a leading UTF-8 byte order mark.
///
/// Returns `(bom, rest)`; `bom` is empty when the string does not start with one.
///
/// Only the UTF-8 BOM is recognised: a `&str` is valid UTF-8 by construction, and
/// the UTF-16 BOM bytes (`0xFE 0xFF`) cannot occur in valid UTF-8, so no other BOM
/// can ever appear here.
pub fn split_bom(s: &str) -> (&str, &str) {
    match s.strip_prefix('\u{feff}') {
        Some(rest) => s.split_at(s.len() - rest.len()),
        None => ("", s),
    }
}

/// Skips spaces and tabs (and nothing else - `\r` is a line ending here, not filler).
pub fn skip_ws(s: &str) -> &str {
    s.trim_start_matches([' ', '\t'])
}

/// Splits a string into `(leading whitespace, trimmed middle, trailing whitespace)`.
///
/// Only `' '` and `'\t'` count as whitespace, so line endings are never eaten. The
/// three pieces concatenate back to the input, which is what makes the `.ssa` writer
/// non-destructive.
pub fn trim_non_destructive(s: &str) -> (&str, &str, &str) {
    let start = s.len() - skip_ws(s).len();
    let rest = &s[start..];
    let trimmed_len = rest.trim_end_matches([' ', '\t']).len();
    (&s[..start], &rest[..trimmed_len], &rest[trimmed_len..])
}

/// Parses an optionally negative run of ASCII digits.
///
/// Returns the value and the unconsumed remainder, or `None` if there is no digit
/// or the number does not fit into an `i64`.
pub fn take_number(s: &str) -> Option<(i64, &str)> {
    let (negative, rest) = match s.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, s),
    };
    let digits_len = rest.bytes().take_while(u8::is_ascii_digit).count();
    if digits_len == 0 {
        return None;
    }
    // A 20-digit run does not fit into an `i64`; report that as a parse error
    // instead of panicking (`subparse 0.7.0` unwrapped here).
    let value: i64 = rest[..digits_len].parse().ok()?;
    Some((if negative { -value } else { value }, &rest[digits_len..]))
}

/// Combines time components into a number of milliseconds.
fn from_components(hours: i64, mins: i64, secs: i64, msecs: i64) -> i64 {
    msecs + 1000 * (secs + 60 * (mins + 60 * hours))
}

/// Consumes the single character `c`, or returns `None`.
fn eat(s: &str, c: char) -> Option<&str> {
    s.strip_prefix(c)
}

/// Parses a `SubRip` timestamp like `00:24:45,670`, returning milliseconds.
fn srt_timestamp(s: &str) -> Option<(i64, &str)> {
    let (hours, s) = take_number(s)?;
    let (mins, s) = take_number(eat(s, ':')?)?;
    let (secs, s) = take_number(eat(s, ':')?)?;
    let (msecs, s) = take_number(eat(s, ',')?)?;
    Some((from_components(hours, mins, secs, msecs), s))
}

/// Parses a whole `SubRip` timespan line like `00:24:45,670 --> 00:24:45,680`.
///
/// Returns `(start, end)` in milliseconds. Leading, trailing and inner spaces and
/// tabs are allowed; anything else on the line makes it fail.
pub fn srt_timespan(line: &str) -> Option<(i64, i64)> {
    let (start, rest) = srt_timestamp(skip_ws(line))?;
    let rest = skip_ws(rest).strip_prefix("-->")?;
    let (end, rest) = srt_timestamp(skip_ws(rest))?;
    if skip_ws(rest).is_empty() {
        Some((start, end))
    } else {
        None
    }
}

/// Parses a `SubStation Alpha` timepoint like `0:19:41.99`, returning milliseconds.
///
/// The separator before the hundredths may be `.` or `:`; the whole string has to be
/// consumed.
pub fn ssa_timepoint(s: &str) -> Option<i64> {
    let (hours, s) = take_number(s)?;
    let (mins, s) = take_number(eat(s, ':')?)?;
    let (secs, s) = take_number(eat(s, ':')?)?;
    let s = eat(s, '.').or_else(|| eat(s, ':'))?;
    let (csecs, s) = take_number(s)?;
    if !s.is_empty() {
        return None;
    }
    Some(from_components(hours, mins, secs, csecs * 10))
}

/// Parses a `VobSub` `.idx` timestamp like `00:41:36:961`, returning milliseconds.
///
/// The whole string has to be consumed.
pub fn idx_timestamp(s: &str) -> Option<i64> {
    let (hours, s) = take_number(s)?;
    let (mins, s) = take_number(eat(s, ':')?)?;
    let (secs, s) = take_number(eat(s, ':')?)?;
    let (msecs, s) = take_number(eat(s, ':')?)?;
    if !s.is_empty() {
        return None;
    }
    Some(from_components(hours, mins, secs, msecs))
}

/// The pieces of an `.idx` `timestamp:` line.
///
/// `(leading whitespace, "timestamp:", whitespace, timestamp, rest of the line)` -
/// concatenating them reproduces the input line exactly.
pub type IdxLineParts<'a> = (&'a str, &'a str, &'a str, &'a str, &'a str);

/// Splits an `.idx` line like `timestamp: 00:41:36:961, filepos: 000000000`.
///
/// Returns `None` when the line does not start with optional spaces/tabs followed
/// by `timestamp:`.
pub fn idx_line(line: &str) -> Option<IdxLineParts<'_>> {
    let (ws1, rest) = line.split_at(line.len() - skip_ws(line).len());
    let rest = rest.strip_prefix("timestamp:")?;
    let (ws2, rest) = rest.split_at(rest.len() - skip_ws(rest).len());
    let timestamp_len = rest.bytes().take_while(|b| b.is_ascii_digit() || *b == b':').count();
    let (timestamp, tail) = rest.split_at(timestamp_len);
    Some((ws1, "timestamp:", ws2, timestamp, tail))
}

/// The pieces of a `.ssa`/`.ass` `Dialogue:` line.
///
/// `(leading whitespace, whitespace after the keyword, the comma-terminated fields,
/// the remaining text field)`.
pub type SsaDialogueParts<'a> = (&'a str, &'a str, Vec<(&'a str, char)>, &'a str);

/// Splits a `.ssa`/`.ass` line like
/// `Dialogue: 1,0:22:43.52,0:22:46.22,ED-Romaji,,0,0,0,,some text`.
///
/// `num_fields` is the number of fields the `Format:` line declared; the first
/// `num_fields - 1` of them are comma terminated and the last one is the rest of the
/// line. Returns `None` if the keyword is missing or there are too few commas.
pub fn ssa_dialogue(line: &str, num_fields: usize) -> Option<SsaDialogueParts<'_>> {
    let (ws1, rest) = line.split_at(line.len() - skip_ws(line).len());
    let rest = rest.strip_prefix("Dialogue:")?;
    let (ws2, mut rest) = rest.split_at(rest.len() - skip_ws(rest).len());

    let mut fields = Vec::with_capacity(num_fields.saturating_sub(1));
    for _ in 0..num_fields.saturating_sub(1) {
        let comma = rest.find(',')?;
        fields.push((&rest[..comma], ','));
        rest = &rest[comma + 1..];
    }
    Some((ws1, ws2, fields, rest))
}

/// One `MicroDVD` sub-line: its `{...}` formatting tags and its text.
pub type MdvdSubLine<'a> = (Vec<&'a str>, &'a str);

/// Parses a `MicroDVD` line like `{0}{25}{y:i}Hello!|{s:15}Hello2!`.
///
/// Returns `(start frame, end frame, sub-lines)`, where the sub-lines are the
/// `'|'`-separated parts, each split into its leading `{...}` formatting tags and
/// the remaining text. An unterminated `{` invalidates the whole line.
pub fn mdvd_line(line: &str) -> Option<(i64, i64, Vec<MdvdSubLine<'_>>)> {
    let rest = line.strip_prefix('{')?;
    let (start_frame, rest) = take_number(rest)?;
    let rest = rest.strip_prefix('}')?.strip_prefix('{')?;
    let (end_frame, rest) = take_number(rest)?;
    let rest = rest.strip_prefix('}')?;

    let mut sub_lines = Vec::new();
    for mut chunk in rest.split('|') {
        let mut formattings = Vec::new();
        while let Some(inner) = chunk.strip_prefix('{') {
            // An unclosed '{' invalidates the line - a partially consumed
            // formatting tag is an error, not the start of the text.
            let close = inner.find('}')?;
            formattings.push(&inner[..close]);
            chunk = &inner[close + 1..];
        }
        sub_lines.push((formattings, chunk));
    }
    Some((start_frame, end_frame, sub_lines))
}

/// Merges consecutive "filler" parts of a non-destructively parsed file.
///
/// Each parsed file holds "filler" parts (unimportant text kept only so the original
/// file can be reconstructed). Two consecutive filler parts can be merged into one;
/// `extract_fn` tells this function which parts are filler and hands out their text.
pub fn dedup_string_parts<T, F>(v: Vec<T>, mut extract_fn: F) -> Vec<T>
where
    F: FnMut(&mut T) -> Option<&mut String>,
{
    let mut result: Vec<T> = Vec::new();
    for mut part in v {
        let mut push_part = true;
        if let Some(last_part) = result.last_mut()
            && let Some(exchangeable_text) = extract_fn(last_part)
            && let Some(new_text) = extract_fn(&mut part)
        {
            exchangeable_text.push_str(new_text);
            push_part = false;
        }

        if push_part {
            result.push(part);
        }
    }

    result
}

/// Splits `s` into lines, keeping each line ending verbatim.
///
/// This makes it possible to reconstruct the file with its original line endings.
/// A lone `\r` is also accepted as a line ending so that no input is ever rejected.
pub fn get_lines_non_destructive(s: &str) -> Vec<SplittedLine> {
    let mut result = Vec::new();
    let mut rest = s;
    loop {
        if rest.is_empty() {
            return result;
        }

        match rest.find(['\r', '\n']) {
            Some(idx) => {
                let (line_str, new_rest) = rest.split_at(idx);
                rest = new_rest;

                let line = line_str.to_string();
                if let Some(new_rest) = rest.strip_prefix("\r\n") {
                    result.push((line, "\r\n".to_string()));
                    rest = new_rest;
                } else if let Some(new_rest) = rest.strip_prefix('\n') {
                    result.push((line, "\n".to_string()));
                    rest = new_rest;
                } else if let Some(new_rest) = rest.strip_prefix('\r') {
                    // Only treated as a line ending to avoid error handling.
                    result.push((line, "\r".to_string()));
                    rest = new_rest;
                }
            }
            None => {
                result.push((rest.to_string(), String::new()));
                return result;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_split_bom() {
        assert_eq!(split_bom("\u{feff}abc"), ("\u{feff}", "abc"));
        assert_eq!(split_bom("\u{feff}"), ("\u{feff}", ""));
        assert_eq!(split_bom("bla"), ("", "bla"));
        assert_eq!(split_bom(""), ("", ""));
        // the UTF-8 BOM is three bytes long
        assert_eq!(split_bom("\u{feff}abc").0.len(), 3);
    }

    #[test]
    fn test_trim_non_destructive() {
        assert_eq!(trim_non_destructive("  hello \t"), ("  ", "hello", " \t"));
        assert_eq!(trim_non_destructive(""), ("", "", ""));
        assert_eq!(trim_non_destructive("   "), ("   ", "", ""));
        assert_eq!(trim_non_destructive("a"), ("", "a", ""));
        // '\r' is not whitespace here
        assert_eq!(trim_non_destructive(" a\r"), (" ", "a\r", ""));

        for input in ["", " ", "a", "  a b  ", "\t\ta\t"] {
            let (b, m, e) = trim_non_destructive(input);
            assert_eq!(format!("{b}{m}{e}"), input);
        }
    }

    #[test]
    fn test_take_number() {
        assert_eq!(take_number("123abc"), Some((123, "abc")));
        assert_eq!(take_number("-7"), Some((-7, "")));
        assert_eq!(take_number("-"), None);
        assert_eq!(take_number("abc"), None);
        assert_eq!(take_number(""), None);
        // does not panic on a number that cannot fit into an i64
        assert_eq!(take_number("99999999999999999999"), None);
    }

    #[test]
    fn test_srt_timespan() {
        assert_eq!(
            srt_timespan("00:24:45,670 --> 00:24:45,680"),
            Some((1_485_670, 1_485_680))
        );
        assert_eq!(srt_timespan("\t0:0:0,0-->0:0:1,0  "), Some((0, 1000)));
        assert_eq!(srt_timespan("00:00:01,000 --> 00:00:02,000 x"), None);
        assert_eq!(srt_timespan("00:00:01.000 --> 00:00:02,000"), None);
        assert_eq!(srt_timespan(""), None);
        assert_eq!(srt_timespan("99999999999999999999:0:0,0 --> 0:0:0,0"), None);
    }

    #[test]
    fn test_ssa_timepoint() {
        assert_eq!(ssa_timepoint("0:19:41.99"), Some(1_181_990));
        assert_eq!(ssa_timepoint("0:19:41:99"), Some(1_181_990));
        assert_eq!(ssa_timepoint("0:19:41,99"), None);
        assert_eq!(ssa_timepoint("0:19:41.99 "), None);
    }

    #[test]
    fn test_idx_timestamp() {
        assert_eq!(idx_timestamp("00:41:36:961"), Some(2_496_961));
        assert_eq!(idx_timestamp("00:41:36.961"), None);
        assert_eq!(idx_timestamp("00:41:36:961x"), None);
    }

    #[test]
    fn test_idx_line() {
        assert_eq!(
            idx_line("  timestamp: 00:00:10:000, filepos: 000000000"),
            Some(("  ", "timestamp:", " ", "00:00:10:000", ", filepos: 000000000"))
        );
        assert_eq!(idx_line("# a comment"), None);
        assert_eq!(idx_line("timestamp:"), Some(("", "timestamp:", "", "", "")));
    }

    #[test]
    fn test_ssa_dialogue() {
        let (ws1, ws2, fields, text) = ssa_dialogue("Dialogue: 0,a,b,rest,of,it", 4).unwrap();
        assert_eq!((ws1, ws2), ("", " "));
        assert_eq!(fields, vec![("0", ','), ("a", ','), ("b", ',')]);
        assert_eq!(text, "rest,of,it");

        assert_eq!(ssa_dialogue("Comment: 0,a,b,c", 4).map(|_| ()), None);
        // too few commas
        assert_eq!(ssa_dialogue("Dialogue: 0,a", 4).map(|_| ()), None);
    }

    #[test]
    fn test_mdvd_line() {
        let (start, end, lines) = mdvd_line("{0}{25}{y:i}Hello!|{s:15}Hello2!").unwrap();
        assert_eq!((start, end), (0, 25));
        assert_eq!(lines, vec![(vec!["y:i"], "Hello!"), (vec!["s:15"], "Hello2!")]);

        assert_eq!(mdvd_line("{0}{25}").unwrap().2, vec![(vec![], "")]);
        // an unclosed '{' invalidates the whole line
        assert!(mdvd_line("{0}{25}{unclosed").is_none());
        assert!(mdvd_line("no braces").is_none());
    }

    #[test]
    fn get_lines_non_destructive_test0() {
        let lines = ["", "aaabb", "aaabb\r\nbcccc\n\r\n ", "aaabb\r\nbcccc", "a\rb"];
        for full_line in lines {
            let joined: String = get_lines_non_destructive(full_line)
                .into_iter()
                .flat_map(|(s1, s2)| [s1, s2])
                .collect();
            assert_eq!(full_line, joined);
        }

        assert_eq!(
            get_lines_non_destructive("a\r\nb\n"),
            vec![
                ("a".to_string(), "\r\n".to_string()),
                ("b".to_string(), "\n".to_string())
            ]
        );
    }
}
