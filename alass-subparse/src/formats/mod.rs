// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! The individual subtitle formats and the dispatch between them.

pub mod common;
pub mod idx;
pub mod microdvd;
pub mod srt;
pub mod ssa;
pub mod vobsub;
pub mod vobsub_timings;

use crate::errors::{Error, Result};
use crate::{SubtitleEntry, SubtitleFileInterface};
use chardetng::{EncodingDetector, Iso2022JpDetection, Utf8Detection};
use encoding_rs::Encoding;
use std::ffi::OsStr;
use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// All formats which are supported by this library.
pub enum SubtitleFormat {
    /// .srt file
    SubRip,

    /// .ssa/.ass file
    SubStationAlpha,

    /// .idx file
    VobSubIdx,

    /// .sub file (`VobSub`/binary)
    VobSubSub,

    /// .sub file (`MicroDVD`/text)
    MicroDVD,
}

#[derive(Clone, Debug)]
/// Unified wrapper around the all individual subtitle file types.
pub enum SubtitleFile {
    /// .srt file
    SubRipFile(srt::SrtFile),

    /// .ssa/.ass file
    SubStationAlpha(ssa::SsaFile),

    /// .idx file
    VobSubIdxFile(idx::IdxFile),

    /// .sub file (`VobSub`/binary)
    VobSubSubFile(vobsub::VobFile),

    /// .sub file (`MicroDVD`/text)
    MicroDVDFile(microdvd::MdvdFile),
}

impl SubtitleFile {
    /// The subtitle entries can be changed by calling `update_subtitle_entries()`.
    pub fn get_subtitle_entries(&self) -> Result<Vec<SubtitleEntry>> {
        match self {
            SubtitleFile::SubRipFile(f) => f.get_subtitle_entries(),
            SubtitleFile::SubStationAlpha(f) => f.get_subtitle_entries(),
            SubtitleFile::VobSubIdxFile(f) => f.get_subtitle_entries(),
            SubtitleFile::VobSubSubFile(f) => f.get_subtitle_entries(),
            SubtitleFile::MicroDVDFile(f) => f.get_subtitle_entries(),
        }
    }

    /// Set the entries from the subtitle entries from the `get_subtitle_entries()`.
    ///
    /// The length of the given input slice should always match the length of the vector length from
    /// `get_subtitle_entries()`. This function can not delete/create new entries, but preserves
    /// everything else in the file (formatting, authors, ...).
    ///
    /// If the input entry has `entry.line == None`, the line will not be overwritten.
    ///
    /// Be aware that .idx files cannot save time_spans_ (a subtitle will be shown between two
    /// consecutive timepoints/there are no separate starts and ends) - so the timepoint will be set
    /// to the start of the corresponding input-timespan.
    pub fn update_subtitle_entries(&mut self, i: &[SubtitleEntry]) -> Result<()> {
        match self {
            SubtitleFile::SubRipFile(f) => f.update_subtitle_entries(i),
            SubtitleFile::SubStationAlpha(f) => f.update_subtitle_entries(i),
            SubtitleFile::VobSubIdxFile(f) => f.update_subtitle_entries(i),
            SubtitleFile::VobSubSubFile(f) => f.update_subtitle_entries(i),
            SubtitleFile::MicroDVDFile(f) => f.update_subtitle_entries(i),
        }
    }

    /// Returns a byte-stream in the respective format (.ssa, .srt, etc.) with the
    /// (probably) altered information.
    pub fn to_data(&self) -> Result<Vec<u8>> {
        match self {
            SubtitleFile::SubRipFile(f) => f.to_data(),
            SubtitleFile::SubStationAlpha(f) => f.to_data(),
            SubtitleFile::VobSubIdxFile(f) => f.to_data(),
            SubtitleFile::VobSubSubFile(f) => f.to_data(),
            SubtitleFile::MicroDVDFile(f) => f.to_data(),
        }
    }
}

impl From<srt::SrtFile> for SubtitleFile {
    fn from(f: srt::SrtFile) -> SubtitleFile {
        SubtitleFile::SubRipFile(f)
    }
}

impl From<ssa::SsaFile> for SubtitleFile {
    fn from(f: ssa::SsaFile) -> SubtitleFile {
        SubtitleFile::SubStationAlpha(f)
    }
}

impl From<idx::IdxFile> for SubtitleFile {
    fn from(f: idx::IdxFile) -> SubtitleFile {
        SubtitleFile::VobSubIdxFile(f)
    }
}

impl From<vobsub::VobFile> for SubtitleFile {
    fn from(f: vobsub::VobFile) -> SubtitleFile {
        SubtitleFile::VobSubSubFile(f)
    }
}

impl From<microdvd::MdvdFile> for SubtitleFile {
    fn from(f: microdvd::MdvdFile) -> SubtitleFile {
        SubtitleFile::MicroDVDFile(f)
    }
}

impl SubtitleFormat {
    /// Get a descriptive string for the format like `".srt (SubRip)"`.
    pub fn get_name(self) -> &'static str {
        match self {
            SubtitleFormat::SubRip => ".srt (SubRip)",
            SubtitleFormat::SubStationAlpha => ".ssa (SubStation Alpha)",
            SubtitleFormat::VobSubIdx => ".idx (VobSub)",
            SubtitleFormat::VobSubSub => ".sub (VobSub)",
            SubtitleFormat::MicroDVD => ".sub (MicroDVD)",
        }
    }
}

impl fmt::Display for SubtitleFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.get_name())
    }
}

/// Returns the subtitle format by the file extension.
///
/// Calling the function with the full file path or simply a `get_subtitle_format_by_extension(Some(OsStr::new("srt")))`
/// both work. Returns `None` if subtitle format could not be recognized.
///
/// Because the `.sub` file extension is ambiguous (both `MicroDVD` and `VobSub` use that extension) the
/// function will return `None` in that case. Instead, use the content-aware `get_subtitle_format`
/// to handle this case correctly.
///
/// `Option` is used to simplify handling with `PathBuf::extension()`.
pub fn get_subtitle_format_by_extension(extension: Option<&OsStr>) -> Option<SubtitleFormat> {
    if extension == Some(OsStr::new("srt")) {
        Some(SubtitleFormat::SubRip)
    } else if extension == Some(OsStr::new("ssa")) || extension == Some(OsStr::new("ass")) {
        Some(SubtitleFormat::SubStationAlpha)
    } else if extension == Some(OsStr::new("idx")) {
        Some(SubtitleFormat::VobSubIdx)
    } else {
        None
    }
}

/// Returns true if the file extension is valid for the given subtitle format.
///
/// `Option` is used to simplify handling with `PathBuf::extension()`.
pub fn is_valid_extension_for_subtitle_format(extension: Option<&OsStr>, format: SubtitleFormat) -> bool {
    match format {
        SubtitleFormat::SubRip => extension == Some(OsStr::new("srt")),
        SubtitleFormat::SubStationAlpha => extension == Some(OsStr::new("ssa")) || extension == Some(OsStr::new("ass")),
        SubtitleFormat::VobSubIdx => extension == Some(OsStr::new("idx")),
        SubtitleFormat::VobSubSub | SubtitleFormat::MicroDVD => extension == Some(OsStr::new("sub")),
    }
}

/// Returns the subtitle format by the file extension.
///
/// Works exactly like `get_subtitle_format_by_extension`, but instead of `None` a `UnknownFileFormat`
/// will be returned (for simpler error handling).
///
/// `Option` is used to simplify handling with `PathBuf::extension()`.
pub fn get_subtitle_format_by_extension_err(extension: Option<&OsStr>) -> Result<SubtitleFormat> {
    get_subtitle_format_by_extension(extension).ok_or(Error::UnknownFileFormat)
}

/// Returns the subtitle format by the file extension and provided content.
///
/// Calling the function with the full file path or simply a `get_subtitle_format(".sub", content)`
/// both work. Returns `None` if subtitle format could not be recognized.
///
/// It works exactly the same as `get_subtitle_format_by_extension` (see documentation), but also handles the `.sub` cases
/// correctly by using the provided content of the file as secondary info.
///
/// `Option` is used to simplify handling with `PathBuf::extension()`.
pub fn get_subtitle_format(extension: Option<&OsStr>, content: &[u8]) -> Option<SubtitleFormat> {
    if extension == Some(OsStr::new("sub")) {
        // test for VobSub .sub magic number
        if content.starts_with(&[0x00, 0x00, 0x01, 0xba]) {
            Some(SubtitleFormat::VobSubSub)
        } else {
            Some(SubtitleFormat::MicroDVD)
        }
    } else {
        get_subtitle_format_by_extension(extension)
    }
}

/// Returns the subtitle format by the file extension and provided content.
///
/// Works exactly like `get_subtitle_format`, but instead of `None` a `UnknownFileFormat`
/// will be returned (for simpler error handling).
pub fn get_subtitle_format_err(extension: Option<&OsStr>, content: &[u8]) -> Result<SubtitleFormat> {
    get_subtitle_format(extension, content).ok_or(Error::UnknownFileFormat)
}

/// Parse text subtitles, invoking the right parser given by `format`.
///
/// Returns an `Err(Error::TextFormatOnly)` if attempted on a binary file format.
///
/// # Mandatory format specific options
///
/// See `parse_bytes`.
pub fn parse_str(format: SubtitleFormat, content: &str, fps: f64) -> Result<SubtitleFile> {
    match format {
        SubtitleFormat::SubRip => Ok(srt::SrtFile::parse(content)?.into()),
        SubtitleFormat::SubStationAlpha => Ok(ssa::SsaFile::parse(content)?.into()),
        SubtitleFormat::VobSubIdx => Ok(idx::IdxFile::parse(content)?.into()),
        SubtitleFormat::VobSubSub => Err(Error::TextFormatOnly),
        SubtitleFormat::MicroDVD => Ok(microdvd::MdvdFile::parse(content, fps)?.into()),
    }
}

/// Helper function for text subtitles for byte-to-text decoding (use `None` for automatic detection).
///
/// An explicitly requested encoding wins; then a byte order mark; only then guesswork.
fn decode_bytes_to_string(content: &[u8], encoding: Option<&'static Encoding>) -> Result<String> {
    // `chardetng` cannot fail, so an empty file would guess its way to an empty string and
    // parse as a subtitle with no lines. That is not a subtitle - and it used to be an error
    // here, back when detection could fail. An explicitly requested encoding still decodes it.
    if encoding.is_none() && content.is_empty() {
        return Err(Error::EncodingDetectionError);
    }

    let encoding = encoding
        .or_else(|| Encoding::for_bom(content).map(|(encoding, _bom_len)| encoding))
        .unwrap_or_else(|| {
            let mut detector = EncodingDetector::new(Iso2022JpDetection::Allow);
            // subtitle files are read into memory in one piece, so this is the only feed
            detector.feed(content, true);
            detector.guess(None, Utf8Detection::Allow)
        });

    let (decoded, _, had_errors) = encoding.decode(content);
    if had_errors {
        Err(Error::DecodingError)
    } else {
        Ok(decoded.into_owned())
    }
}

/// Parse all subtitle formats, invoking the right parser given by `format`.
///
/// # Mandatory format specific options
///
/// Some subtitle formats require additional parameters to work as expected. If you want to parse
/// a specific format that has no additional parameters, you can use the `parse` function of
/// the respective `***File` struct.
///
/// `encoding`: to parse a text-based subtitle format, a character encoding is needed (use `None`
/// to sniff a byte order mark and otherwise auto-detect with `chardetng`)
///
/// `fps`: this parameter is used for `MicroDVD` `.sub` files. These files do not store timestamps in
/// seconds/minutes/... but in frame numbers. So the timing `0 to 30` means "show subtitle for one second"
/// for a 30fps video, and "show subtitle for half second" for 60fps videos. The parameter specifies how
/// frame numbers are converted into timestamps.
pub fn parse_bytes(
    format: SubtitleFormat,
    content: &[u8],
    encoding: Option<&'static Encoding>,
    fps: f64,
) -> Result<SubtitleFile> {
    match format {
        SubtitleFormat::SubRip => Ok(srt::SrtFile::parse(&decode_bytes_to_string(content, encoding)?)?.into()),
        SubtitleFormat::SubStationAlpha => Ok(ssa::SsaFile::parse(&decode_bytes_to_string(content, encoding)?)?.into()),
        SubtitleFormat::VobSubIdx => Ok(idx::IdxFile::parse(&decode_bytes_to_string(content, encoding)?)?.into()),
        SubtitleFormat::VobSubSub => Ok(vobsub::VobFile::parse(content)?.into()),
        SubtitleFormat::MicroDVD => {
            Ok(microdvd::MdvdFile::parse(&decode_bytes_to_string(content, encoding)?, fps)?.into())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_subtitle_format_by_extension() {
        // this shows how the input parameter can be created from scratch
        assert_eq!(
            get_subtitle_format_by_extension(Some(OsStr::new("srt"))),
            Some(SubtitleFormat::SubRip)
        );
        assert_eq!(
            get_subtitle_format_by_extension(Some(OsStr::new("ass"))),
            Some(SubtitleFormat::SubStationAlpha)
        );
        assert_eq!(
            get_subtitle_format_by_extension(Some(OsStr::new("idx"))),
            Some(SubtitleFormat::VobSubIdx)
        );
        // `.sub` is ambiguous without the content
        assert_eq!(get_subtitle_format_by_extension(Some(OsStr::new("sub"))), None);
        assert_eq!(get_subtitle_format_by_extension(None), None);
    }

    #[test]
    fn sub_files_are_told_apart_by_their_magic_number() {
        assert_eq!(
            get_subtitle_format(Some(OsStr::new("sub")), &[0x00, 0x00, 0x01, 0xba, 0x44]),
            Some(SubtitleFormat::VobSubSub)
        );
        assert_eq!(
            get_subtitle_format(Some(OsStr::new("sub")), b"{0}{25}Hi"),
            Some(SubtitleFormat::MicroDVD)
        );
        // a file shorter than the magic number is not VobSub
        assert_eq!(
            get_subtitle_format(Some(OsStr::new("sub")), &[0x00, 0x00]),
            Some(SubtitleFormat::MicroDVD)
        );
    }

    #[test]
    fn extension_validation() {
        assert!(is_valid_extension_for_subtitle_format(
            Some(OsStr::new("ssa")),
            SubtitleFormat::SubStationAlpha
        ));
        assert!(is_valid_extension_for_subtitle_format(
            Some(OsStr::new("sub")),
            SubtitleFormat::MicroDVD
        ));
        assert!(!is_valid_extension_for_subtitle_format(
            Some(OsStr::new("srt")),
            SubtitleFormat::VobSubIdx
        ));
    }

    #[test]
    fn unknown_formats_are_reported() {
        assert!(get_subtitle_format_by_extension_err(Some(OsStr::new("txt"))).is_err());
        assert!(get_subtitle_format_err(Some(OsStr::new("txt")), b"").is_err());
    }

    #[test]
    fn explicit_encoding_wins() {
        // "Café" in windows-1252
        let bytes = b"Caf\xe9";
        assert_eq!(
            decode_bytes_to_string(bytes, Some(encoding_rs::WINDOWS_1252)).unwrap(),
            "Café"
        );
        // the same bytes are not valid UTF-8
        assert!(decode_bytes_to_string(bytes, Some(encoding_rs::UTF_8)).is_err());
    }

    #[test]
    fn a_byte_order_mark_is_honoured_before_guessing() {
        let utf16le = b"\xff\xfeh\0e\0l\0l\0o\0";
        assert_eq!(decode_bytes_to_string(utf16le, None).unwrap(), "hello");

        let utf8_bom = b"\xef\xbb\xbfhello";
        assert_eq!(decode_bytes_to_string(utf8_bom, None).unwrap(), "hello");
    }

    #[test]
    fn windows_1251_is_detected_as_cyrillic() {
        // "Привет, как дела" in windows-1251; `chardet` used to guess x-mac-cyrillic
        // here and decoded the first letter as "ѕ".
        let bytes = b"\xcf\xf0\xe8\xe2\xe5\xf2, \xea\xe0\xea \xe4\xe5\xeb\xe0";
        assert_eq!(decode_bytes_to_string(bytes, None).unwrap(), "Привет, как дела");
    }

    /// `chardetng` only ever guesses ASCII-compatible encodings, so UTF-16 is reached
    /// through its BOM. Without one the text comes out full of NULs and the parse
    /// fails - which is also what `subparse 0.7.0` did with this input.
    #[test]
    fn utf16_without_a_bom_is_not_silently_mojibaked() {
        let mut utf16 = Vec::new();
        for unit in "1\n00:00:10,000 --> 00:00:12,000\nHello\n\n".encode_utf16() {
            utf16.extend_from_slice(&unit.to_le_bytes());
        }
        assert!(parse_bytes(SubtitleFormat::SubRip, &utf16, None, 25.0).is_err());
    }

    /// Realistic Western-European and CJK subtitle text has to survive detection: the
    /// output writers always emit UTF-8, so a wrong guess is written back as mojibake.
    #[test]
    fn realistic_subtitle_text_survives_detection() {
        for (bytes, expected) in [
            (
                // Polish in windows-1250; `chardet` used to decode this as windows-1252
                &b"Nie mog\xea uwierzy\xe6, \xbfe on ju\xbf wyjecha\xb3 st\xb9d."[..],
                "Nie mogę uwierzyć, że on już wyjechał stąd.",
            ),
            (
                // German in windows-1252
                &b"Die Stra\xdfe war v\xf6llig \xfcberf\xfcllt heute Abend."[..],
                "Die Straße war völlig überfüllt heute Abend.",
            ),
            (
                // the same sentence in UTF-8
                "Die Straße war völlig überfüllt heute Abend.".as_bytes(),
                "Die Straße war völlig überfüllt heute Abend.",
            ),
            (
                // Simplified Chinese in GBK
                &b"\xce\xd2\xd5\xe6\xb2\xbb\xb8\xd2\xcf\xe0\xd0\xc5\xcb\xfb\xd2\xd1\xbe\xad\xc0\xeb\xbf\xaa\xd5\xe2\xc0\xef\xc1\xcb\xa1\xa3"[..],
                "我真不敢相信他已经离开这里了。",
            ),
        ] {
            assert_eq!(decode_bytes_to_string(bytes, None).unwrap(), expected);
        }
    }

    #[test]
    fn an_empty_file_has_no_detectable_encoding() {
        assert!(matches!(
            decode_bytes_to_string(b"", None),
            Err(Error::EncodingDetectionError)
        ));
        assert_eq!(decode_bytes_to_string(b"", Some(encoding_rs::UTF_8)).unwrap(), "");
    }

    #[test]
    fn parse_str_rejects_binary_formats() {
        assert_eq!(
            parse_str(SubtitleFormat::VobSubSub, "", 25.0).unwrap_err().to_string(),
            "operation does not work on binary subtitle formats (only text formats)"
        );
    }

    #[test]
    fn parse_bytes_dispatches_to_every_format() {
        let srt = parse_bytes(
            SubtitleFormat::SubRip,
            b"1\n00:00:01,000 --> 00:00:02,000\nhi\n\n",
            None,
            25.0,
        )
        .unwrap();
        assert!(matches!(srt, SubtitleFile::SubRipFile(_)));
        assert_eq!(srt.get_subtitle_entries().unwrap().len(), 1);

        let mdvd = parse_bytes(SubtitleFormat::MicroDVD, b"{0}{25}hi", None, 25.0).unwrap();
        assert!(matches!(mdvd, SubtitleFile::MicroDVDFile(_)));

        let sub = parse_bytes(
            SubtitleFormat::VobSubSub,
            include_bytes!("../../fixtures/tiny.sub"),
            None,
            25.0,
        )
        .unwrap();
        assert!(matches!(sub, SubtitleFile::VobSubSubFile(_)));
        assert_eq!(sub.get_subtitle_entries().unwrap().len(), 1);
    }

    #[test]
    fn an_empty_file_is_not_a_subtitle_with_no_lines() {
        // `chardetng` always returns some encoding, so without this guard an empty file
        // would parse into zero entries and align into a silent copy of its counterpart.
        assert!(matches!(
            parse_bytes(SubtitleFormat::SubRip, b"", None, 25.0),
            Err(Error::EncodingDetectionError)
        ));

        // an explicitly requested encoding still decodes it, as it always did
        assert!(parse_bytes(SubtitleFormat::SubRip, b"", Some(encoding_rs::UTF_8), 25.0).is_ok());
    }
}
