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

use alass_subparse::SubtitleFormat;
use std::path::PathBuf;
use thiserror::Error;

/// Something went wrong while reading the file the timings are taken from.
#[derive(Debug, Error)]
pub enum InputFileError {
    #[error("processing video file '{}' failed", path.display())]
    VideoFile {
        path: PathBuf,
        #[source]
        source: InputVideoError,
    },
    #[error("processing subtitle file '{}' failed", path.display())]
    SubtitleFile {
        path: PathBuf,
        #[source]
        source: InputSubtitleError,
    },
}

/// Something went wrong while reading or writing a file.
#[derive(Debug, Error)]
pub enum FileOperationError {
    #[error("failed to open file '{}'", path.display())]
    FileOpen {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to read file '{}'", path.display())]
    FileRead {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to write file '{}'", path.display())]
    FileWrite {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

/// Something went wrong while extracting the voice segments from a video file.
#[derive(Debug, Error)]
pub enum InputVideoError {
    #[error("failed to extract voice segments from file '{}'", path.display())]
    FailedToDecode {
        path: PathBuf,
        /// boxed because a decoder error carries the whole command line it failed on
        #[source]
        source: Box<crate::video_decoder::DecoderError>,
    },
    #[error("failed to analyse audio segment for voice activity")]
    VadAnalysisFailed,
}

/// Something went wrong while reading the timings out of a subtitle file.
#[derive(Debug, Error)]
pub enum InputSubtitleError {
    #[error("reading subtitle file '{}' failed", path.display())]
    ReadingSubtitleFileFailed {
        path: PathBuf,
        #[source]
        source: FileOperationError,
    },
    #[error("unknown subtitle format for file '{}'", path.display())]
    UnknownSubtitleFormat {
        path: PathBuf,
        #[source]
        source: alass_subparse::errors::Error,
    },
    #[error("parsing subtitle file '{}' failed", path.display())]
    ParsingSubtitleFailed {
        path: PathBuf,
        #[source]
        source: alass_subparse::errors::Error,
    },
    #[error("retrieving subtitle lines of file '{}' failed", path.display())]
    RetrievingSubtitleLinesFailed {
        path: PathBuf,
        #[source]
        source: alass_subparse::errors::Error,
    },
}

/// The failures that end the run outright.
#[derive(Debug, Error)]
pub enum TopLevelError {
    #[error(
        "output file '{}' seems to have a different format than input file '{}' with format '{}' (this program does not perform conversions)",
        output_file_path.display(),
        input_file_path.display(),
        input_file_format.get_name()
    )]
    FileFormatMismatch {
        input_file_path: PathBuf,
        output_file_path: PathBuf,
        input_file_format: SubtitleFormat,
    },
    #[error("failed to change lines in the subtitle")]
    FailedToUpdateSubtitle(#[source] alass_subparse::errors::Error),
    #[error("failed to generate data for subtitle")]
    FailedToGenerateSubtitleData(#[source] alass_subparse::errors::Error),
    #[error("failed to instantiate subtitle file")]
    FailedToInstantiateSubtitleFile(#[source] alass_subparse::errors::Error),
}
