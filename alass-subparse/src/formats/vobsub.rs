// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! Binary `VobSub` (`.sub`) timing extraction.

use crate::errors::{Error, ParseError, Result as SubtitleParserResult, VobSubError};
use crate::formats::vobsub_timings;
use crate::timetypes::{TimePoint, TimeSpan};
use crate::{SubtitleEntry, SubtitleFileInterface, SubtitleFormat};

#[derive(Debug, Clone)]
/// Represents a `.sub` (`VobSub`) file.
pub struct VobFile {
    /// Saves the file data.
    data: Vec<u8>,

    /// The extracted subtitle timings.
    lines: Vec<VobSubSubtitle>,
}

#[derive(Debug, Clone, Copy)]
/// Represents a line in a `VobSub` `.sub` file.
struct VobSubSubtitle {
    timespan: TimeSpan,
}

impl VobFile {
    /// Parse contents of a `VobSub` `.sub` file to `VobFile`.
    pub fn parse(b: &[u8]) -> SubtitleParserResult<Self> {
        let lines = vobsub_timings::timings(b)
            .map_err(|cause| Error::from(ParseError::from(VobSubError::from(cause))))?
            .into_iter()
            // only the timestamps are kept; the RLE bitmap is never decoded
            .map(|t| VobSubSubtitle {
                timespan: TimeSpan {
                    start: TimePoint::from_msecs((t.start_secs * 1000.0) as i64),
                    end: TimePoint::from_msecs((t.end_secs * 1000.0) as i64),
                },
            })
            .collect();

        Ok(VobFile {
            data: b.to_vec(),
            lines,
        })
    }
}

impl SubtitleFileInterface for VobFile {
    fn get_subtitle_entries(&self) -> SubtitleParserResult<Vec<SubtitleEntry>> {
        Ok(self
            .lines
            .iter()
            .map(|vsub| SubtitleEntry {
                timespan: vsub.timespan,
                line: None,
            })
            .collect())
    }

    fn update_subtitle_entries(&mut self, _: &[SubtitleEntry]) -> SubtitleParserResult<()> {
        Err(Error::UpdatingEntriesNotSupported {
            format: SubtitleFormat::VobSubSub,
        })
    }

    fn to_data(&self) -> SubtitleParserResult<Vec<u8>> {
        Ok(self.data.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::VobFile;
    use crate::SubtitleFileInterface;

    #[test]
    fn example_fixture_timings() {
        let data = include_bytes!("../../fixtures/example.sub");
        let file = VobFile::parse(data).unwrap();
        let entries = file.get_subtitle_entries().unwrap();
        let ms: Vec<_> = entries
            .iter()
            .map(|e| (e.timespan.start.msecs(), e.timespan.end.msecs()))
            .collect();
        assert_eq!(ms, [(49466, 50966), (52635, 55565)]);
        assert!(entries.iter().all(|e| e.line.is_none()));
    }

    #[test]
    fn tiny_fixtures_agree() {
        let tiny = VobFile::parse(include_bytes!("../../fixtures/tiny.sub")).unwrap();
        let split = VobFile::parse(include_bytes!("../../fixtures/tiny-split.sub")).unwrap();
        let spans = |f: &VobFile| {
            f.get_subtitle_entries()
                .unwrap()
                .iter()
                .map(|e| (e.timespan.start.msecs(), e.timespan.end.msecs()))
                .collect::<Vec<_>>()
        };
        assert_eq!(spans(&tiny), [(1000, 2740)]);
        assert_eq!(spans(&split), spans(&tiny));
    }

    #[test]
    fn to_data_returns_the_original_bytes() {
        let data = include_bytes!("../../fixtures/tiny.sub");
        let file = VobFile::parse(data).unwrap();
        assert_eq!(file.to_data().unwrap(), data.as_slice());
    }

    #[test]
    fn updating_entries_is_not_supported() {
        let mut file = VobFile::parse(include_bytes!("../../fixtures/tiny.sub")).unwrap();
        let error = file.update_subtitle_entries(&[]).unwrap_err();
        assert_eq!(
            error.to_string(),
            "updating subtitles is not implemented or supported by the `subparse` library for this format: .sub (VobSub)"
        );
    }
}
