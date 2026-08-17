// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! The error types of this crate.
//!
//! Everything here implements [`std::error::Error`], and every wrapping error
//! reports the error it wraps through [`std::error::Error::source()`], so a caller
//! can print the full chain.

use crate::SubtitleFormat;
use thiserror::Error;

/// A result type that can be used crate-wide for error handling.
pub type Result<T> = std::result::Result<T, Error>;

/// Anything that can go wrong while reading or writing a subtitle file.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum Error {
    /// The subtitle data could not be parsed; the [`source`](std::error::Error::source)
    /// says why.
    #[error("parsing the subtitle data failed")]
    ParsingError(#[from] ParseError),

    /// The file format is not supported by this library.
    #[error(
        "unknown file format, only SubRip (.srt), SubStationAlpha (.ssa/.ass) and VobSub (.idx and .sub) are supported at the moment"
    )]
    UnknownFileFormat,

    /// The bytes are not valid text in the character encoding that was used.
    #[error("error while decoding subtitle from bytes to string (wrong charset encoding?)")]
    DecodingError,

    /// The character encoding of the subtitle data could not be determined.
    #[error("could not determine character encoding from byte array (manually supply character encoding?)")]
    EncodingDetectionError,

    /// The attempted operation does not work on binary subtitle formats.
    #[error("operation does not work on binary subtitle formats (only text formats)")]
    TextFormatOnly,

    /// The attempted operation does not work on this format (not supported in this version of this library).
    #[error("updating subtitles is not implemented or supported by the `subparse` library for this format: {format}")]
    UpdatingEntriesNotSupported {
        /// The format for which updating the subtitle entries is not supported.
        format: SubtitleFormat,
    },

    /// `update_subtitle_entries()` was called with a slice whose length does not
    /// match the number of entries in the file.
    #[error("the subtitle file has {expected} entries, but {provided} were provided")]
    EntryCountMismatch {
        /// How many entries the file has.
        expected: usize,
        /// How many entries the caller passed in.
        provided: usize,
    },
}

/// Why a subtitle file could not be parsed, per format.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum ParseError {
    /// A `.srt` file could not be parsed.
    #[error(transparent)]
    SubRip(#[from] SubRipError),

    /// A `.ssa`/`.ass` file could not be parsed.
    #[error(transparent)]
    SubStationAlpha(#[from] SsaError),

    /// A `.idx` file could not be parsed.
    #[error(transparent)]
    VobSubIdx(#[from] IdxError),

    /// A binary `VobSub` `.sub` file could not be parsed.
    #[error(transparent)]
    VobSubSub(#[from] VobSubError),

    /// A `MicroDVD` `.sub` file could not be parsed.
    #[error(transparent)]
    MicroDvd(#[from] MicroDvdError),
}

/// A `.srt` parse failure together with the line it happened on.
#[derive(Debug, Error)]
#[error("parse error at line `{line_num}`")]
pub struct SubRipError {
    /// The zero-based index of the offending line.
    pub line_num: usize,

    /// What was wrong with that line.
    #[source]
    pub kind: SubRipErrorKind,
}

/// The concrete problem found in a `.srt` line.
#[derive(Debug, Error, PartialEq, Eq)]
#[non_exhaustive]
pub enum SubRipErrorKind {
    /// A line that should have held the subtitle number did not.
    #[error("expected SubRip index line, found '{line}'")]
    ExpectedIndexLine {
        /// The offending line.
        line: String,

        /// Why the line is not a number. `subparse 0.7.0` reported this as the last
        /// link of its `failure` chain, so `alass-cli` printed it as a fourth
        /// `caused by:` line; keeping it preserves that output.
        #[source]
        cause: std::num::ParseIntError,
    },

    /// A line that should have held `start --> end` did not.
    #[error("expected SubRip timespan line, found '{line}'")]
    ExpectedTimestampLine {
        /// The offending line.
        line: String,
    },
}

/// Everything that can go wrong while parsing a `.ssa`/`.ass` file.
#[derive(Debug, Error, PartialEq, Eq)]
#[non_exhaustive]
pub enum SsaError {
    /// No `Format:` line was found in the `[Events]` section.
    #[error(".ssa/.ass file did not have a line beginning with `Format: ` in a `[Events]` section")]
    SsaFieldsInfoNotFound,

    /// The `Format:` line does not declare a field this library needs.
    #[error("the '{f}' field is missing in the field info in line {line_num}")]
    SsaMissingField {
        /// The zero-based index of the `Format:` line.
        line_num: usize,
        /// The name of the missing field.
        f: &'static str,
    },

    /// The `Format:` line declares a field twice.
    #[error("the '{f}' field is twice in the field info in line {line_num}")]
    SsaDuplicateField {
        /// The zero-based index of the `Format:` line.
        line_num: usize,
        /// The name of the duplicated field.
        f: &'static str,
    },

    /// The `Format:` line does not end with `Text`.
    #[error("the field info in line {line_num} has to have `Text` as its last field")]
    SsaTextFieldNotLast {
        /// The zero-based index of the `Format:` line.
        line_num: usize,
    },

    /// A timepoint field is not of the form `0:19:41.99`.
    #[error("the timepoint `{string}` in line {line_num} has wrong format")]
    SsaWrongTimepointFormat {
        /// The zero-based index of the offending line.
        line_num: usize,
        /// The text that should have been a timepoint.
        string: String,
    },

    /// A `Dialogue:` line does not match the declared `Format:`.
    #[error("parsing the line `{line_num}` failed because of `{msg}`")]
    SsaDialogLineParseError {
        /// The zero-based index of the offending line.
        line_num: usize,
        /// A description of what was expected.
        msg: String,
    },
}

/// Everything that can go wrong while parsing a `.idx` file.
#[derive(Debug, Error, PartialEq, Eq)]
#[non_exhaustive]
pub enum IdxError {
    /// A `timestamp:` line could not be parsed.
    #[error("parsing the line `{line_num}` failed because of `{msg}`")]
    IdxLineParseError {
        /// The zero-based index of the offending line.
        line_num: usize,
        /// A description of what was expected.
        msg: String,
    },
}

/// A `MicroDVD` `.sub` parse failure together with the line it happened on.
#[derive(Debug, Error)]
#[error("parse error at line `{line_num}`")]
pub struct MicroDvdError {
    /// The zero-based index of the offending line.
    pub line_num: usize,

    /// What was wrong with that line.
    #[source]
    pub kind: MicroDvdErrorKind,
}

/// The concrete problem found in a `MicroDVD` `.sub` line.
#[derive(Debug, Error, PartialEq, Eq)]
#[non_exhaustive]
pub enum MicroDvdErrorKind {
    /// A line is not of the form `{start}{end}text`.
    #[error("expected subtitle line, found `{line}`")]
    ExpectedSubtitleLine {
        /// The offending line.
        line: String,
    },
}

/// A failure while reading timings out of a binary `VobSub` `.sub` stream.
#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
#[error("VobSub error: {0}")]
pub struct VobSubError(pub VobSubStreamError);

impl From<VobSubStreamError> for VobSubError {
    fn from(cause: VobSubStreamError) -> Self {
        Self(cause)
    }
}

/// The concrete problem found in the MPEG-2 program stream of a binary `.sub` file.
///
/// The wording of these messages is inherited from `vobsub 0.2.3`.
#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum VobSubStreamError {
    /// The stream ends in the middle of a PES packet.
    #[error("incomplete PES packet")]
    IncompletePesPacket,

    /// A subpicture packet carries no presentation timestamp.
    #[error("found subtitle without timing info")]
    SubtitleWithoutTiming,

    /// A subpicture packet is too short to even hold its own length.
    #[error("packet is too short")]
    PacketTooShort,

    /// A subpicture packet ends before its size field.
    #[error("unexpected end of subtitle data")]
    UnexpectedEndOfSubtitleData,

    /// The control block starts past the end of the subpicture packet.
    #[error("control offset is 0x{offset:x}, but packet is only 0x{len:x} bytes")]
    ControlOffsetOutOfBounds {
        /// The offset the packet asked for.
        offset: usize,
        /// The length of the packet.
        len: usize,
    },

    /// A control sequence points backwards, which would loop forever.
    #[error("control offset went backwards")]
    ControlOffsetWentBackwards,

    /// A control sequence ends without its `0xff` terminator.
    #[error("incomplete control packet")]
    IncompleteControlSequence,

    /// No control command set the subtitle's start time.
    #[error("no start time for subtitle")]
    MissingStartTime,

    /// No control command set the subtitle's position.
    #[error("no coordinates for subtitle")]
    MissingCoordinates,

    /// No control command set the subtitle's palette.
    #[error("no palette for subtitle")]
    MissingPalette,

    /// No control command set the subtitle's alpha channel.
    #[error("no alpha for subtitle")]
    MissingAlpha,

    /// No control command set the offsets of the bitmap scan lines.
    #[error("no RLE offsets for subtitle")]
    MissingRleOffsets,

    /// The subtitle's bounding box has a non-positive width or height.
    #[error("invalid bounding box")]
    InvalidBoundingBox,

    /// The bitmap scan lines start after they end, or past the end of the packet.
    #[error("invalid scan line offsets")]
    InvalidScanLineOffsets,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::error::Error as _;

    #[test]
    fn error_chain_is_walkable() {
        let error = Error::from(ParseError::from(SubRipError {
            line_num: 3,
            kind: SubRipErrorKind::ExpectedIndexLine {
                line: "x".to_string(),
                cause: "x".parse::<i64>().unwrap_err(),
            },
        }));

        let mut chain = vec![error.to_string()];
        let mut source = error.source();
        while let Some(cause) = source {
            chain.push(cause.to_string());
            source = cause.source();
        }

        // Exactly the four links `subparse 0.7.0` produced through `failure`, which
        // `alass-cli` prints as `error:` plus three `caused by:` lines.
        assert_eq!(
            chain,
            vec![
                "parsing the subtitle data failed".to_string(),
                "parse error at line `3`".to_string(),
                "expected SubRip index line, found 'x'".to_string(),
                "invalid digit found in string".to_string(),
            ]
        );
    }

    #[test]
    fn error_is_send_and_sync() {
        fn assert_send_sync<T: Send + Sync + 'static>() {}
        assert_send_sync::<Error>();
    }

    #[test]
    fn top_level_messages_are_unchanged() {
        assert_eq!(
            Error::UnknownFileFormat.to_string(),
            "unknown file format, only SubRip (.srt), SubStationAlpha (.ssa/.ass) and VobSub (.idx and .sub) are supported at the moment"
        );
        assert_eq!(
            Error::UpdatingEntriesNotSupported {
                format: SubtitleFormat::VobSubSub
            }
            .to_string(),
            "updating subtitles is not implemented or supported by the `subparse` library for this format: .sub (VobSub)"
        );
    }
}
