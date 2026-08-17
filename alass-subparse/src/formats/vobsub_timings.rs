// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! Minimal `VobSub` (`.sub`) timing extractor.
//!
//! A `.sub` file is an MPEG-2 Program Stream carrying one or more DVD subpicture
//! substreams. This crate only needs the presentation timestamps, so this module
//! parses the PS/PES framing and the subpicture *control* sequences and deliberately
//! stops before the run-length-encoded bitmap payload.
//!
//! This is a port of the timing half of `vobsub 0.2.3` (CC0-1.0, Eric Kidd,
//! <https://github.com/emk/subtitles-rs>) with its `nom`/`image`/`regex`/`safemem`/
//! `error-chain` dependencies removed. It reproduces `vobsub`'s timings bit for bit
//! on that project's fixtures, including the split-packet and truncated-stream
//! cases; see the tests at the bottom of this file.

use crate::errors::VobSubStreamError as Error;

/// Start and end of one subpicture, in seconds.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VobSubTiming {
    /// When the subpicture appears.
    pub start_secs: f64,

    /// When the subpicture disappears.
    pub end_secs: f64,
}

/// If a subpicture carries no stop date it ends just before the next one starts,
/// this much earlier.
const DEFAULT_SUBTITLE_SPACING: f64 = 0.001;

/// ...but never lasts longer than this.
const DEFAULT_SUBTITLE_LENGTH: f64 = 5.0;

/// Program Stream pack start code.
const PS_PACK_START: [u8; 4] = [0x00, 0x00, 0x01, 0xba];

/// PES start code of `private_stream_1`, which is what DVD subpictures use.
const PES_PRIVATE_STREAM_1: [u8; 4] = [0x00, 0x00, 0x01, 0xbd];

/// Reads a big-endian `u16` from the first two bytes of `b`.
fn be_u16(b: &[u8]) -> u16 {
    u16::from(b[0]) << 8 | u16::from(b[1])
}

/// One PES payload plus the presentation timestamp of the packet that carried it.
#[derive(Debug)]
struct PesPayload<'a> {
    pts_secs: Option<f64>,
    substream_id: u8,
    data: &'a [u8],
}

/// Tests the bits selected by `mask` in `bytes[index]`, for a marker bit that has to
/// be set.
///
/// `vobsub` spells these out as `tag_bits!`, and a mismatch makes it discard the pack.
/// Accepting a pack it would have rejected can turn a recoverable file into an error
/// further down, so they are checked here too.
fn marker(bytes: &[u8], index: usize, mask: u8) -> bool {
    bytes.get(index).is_some_and(|b| b & mask == mask)
}

/// Reads the 33-bit, 90 kHz presentation timestamp out of a PES header data field.
///
/// The field is five bytes: a four-bit tag, then 33 value bits interleaved with three
/// marker bits, each of which has to be 1.
fn read_pts_secs(b: &[u8]) -> Option<f64> {
    let b: [u8; 5] = b.get(..5)?.try_into().ok()?;
    if !(marker(&b, 0, 0x01) && marker(&b, 2, 0x01) && marker(&b, 4, 0x01)) {
        return None;
    }
    let raw =
        u64::from(b[0]) << 32 | u64::from(b[1]) << 24 | u64::from(b[2]) << 16 | u64::from(b[3]) << 8 | u64::from(b[4]);
    let hi = (raw >> 33) & 0x7;
    let mid = (raw >> 17) & 0x7fff;
    let lo = (raw >> 1) & 0x7fff;
    Some((hi << 30 | mid << 15 | lo) as f64 / 90_000.0)
}

/// The marker bits of the 10-byte MPEG-2 pack header - the four inside the system
/// clock reference and the pair after the bit rate - in the order `vobsub` reads them.
const PACK_HEADER_MARKERS: [(usize, u8); 5] = [(0, 0x04), (2, 0x04), (4, 0x04), (5, 0x01), (8, 0x03)];

/// Iterator over the `private_stream_1` PES payloads of a Program Stream.
#[derive(Debug)]
struct PesPackets<'a> {
    remaining: &'a [u8],
}

/// What one attempt at parsing a pack at the current position produced.
enum PackOutcome<'a> {
    /// A payload, plus everything after the packet it came from.
    Packet(PesPayload<'a>, &'a [u8]),

    /// This looked like a pack but did not parse; resynchronise past the start code.
    Resync,
}

impl<'a> Iterator for PesPackets<'a> {
    type Item = Result<PesPayload<'a>, Error>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            // Search for the start of a Program Stream pack.
            let start = self.remaining.windows(4).position(|w| w == PS_PACK_START)?;
            self.remaining = &self.remaining[start..];

            match parse_pack(self.remaining) {
                Ok(PackOutcome::Packet(payload, after)) => {
                    self.remaining = after;
                    return Some(Ok(payload));
                }
                // Same recovery as `vobsub 0.2.3`: skip the start code and keep
                // hunting from just after it.
                Ok(PackOutcome::Resync) => self.remaining = &self.remaining[4..],
                Err(e) => {
                    self.remaining = &[];
                    return Some(Err(e));
                }
            }
        }
    }
}

/// Parses one PS pack header plus the PES packet inside it.
///
/// `buf` must start with [`PS_PACK_START`]. An `Err` means the stream is truncated
/// and there is nothing left to recover.
fn parse_pack(buf: &[u8]) -> Result<PackOutcome<'_>, Error> {
    // 4 start-code bytes, then a 10-byte pack header. `vobsub` walks that header bit
    // by bit, so a byte that is simply missing means a truncated stream while a tag
    // or marker bit that is present but wrong only makes it resynchronise past this
    // pack. Checking them in the same order keeps both answers: demanding the whole
    // header up front would report a truncated file for four bytes of trailing
    // garbage that happen to look like a start code.
    let header = buf.get(4..).unwrap_or_default();
    match header.first() {
        None => return Err(Error::IncompletePesPacket),
        Some(&version) if version >> 6 != 0b01 => return Ok(PackOutcome::Resync), // not MPEG-2
        Some(_) => {}
    }
    for (index, mask) in PACK_HEADER_MARKERS {
        match header.get(index) {
            None => return Err(Error::IncompletePesPacket),
            Some(b) if b & mask != mask => return Ok(PackOutcome::Resync),
            Some(_) => {}
        }
    }
    let Some(header) = header.get(..10) else {
        return Err(Error::IncompletePesPacket);
    };
    let stuffing = usize::from(header[9] & 0b111);
    let Some(rest) = buf.get(14 + stuffing..) else {
        return Err(Error::IncompletePesPacket);
    };

    // Only `private_stream_1` carries DVD subpictures. The start code has to be
    // tested before the length, and a short buffer only counts as "truncated" when
    // the bytes that *are* there match: `vobsub` matches this with nom's `tag!`,
    // which reports "incomplete" for a matching prefix but a plain mismatch - and so
    // resynchronises - as soon as one byte differs. Testing the length first would
    // reject a whole file because of a short trailing pack that is not a subtitle.
    let common = rest.len().min(PES_PRIVATE_STREAM_1.len());
    if rest[..common] != PES_PRIVATE_STREAM_1[..common] {
        return Ok(PackOutcome::Resync);
    }
    if rest.len() < 6 {
        return Err(Error::IncompletePesPacket);
    }
    let pes_len = usize::from(be_u16(&rest[4..6]));
    let rest = &rest[6..];
    if rest.len() < pes_len {
        return Err(Error::IncompletePesPacket);
    }
    let (body, after) = rest.split_at(pes_len);

    // body[0]: '10' + scrambling(2) + priority + alignment + copyright + original
    if body.len() < 3 || body[0] >> 6 != 0b10 {
        return Ok(PackOutcome::Resync);
    }
    let pts_dts_flags = body[1] >> 6;
    let header_data_len = usize::from(body[2]);
    let Some(header_data) = body.get(3..3 + header_data_len) else {
        return Ok(PackOutcome::Resync);
    };
    let Some(&substream_id) = body.get(3 + header_data_len) else {
        return Ok(PackOutcome::Resync);
    };

    let pts_secs = match pts_dts_flags {
        // This packet carries no timestamps at all. `vobsub` parses that happily and
        // only complains later, when it turns out to be the first packet of a
        // subpicture; `timings()` below does the same.
        0b00 => None,
        // PTS only
        0b10 if header_data.first().is_some_and(|b| b >> 4 == 0b0010) => read_pts_secs(header_data),
        // PTS and DTS; the PTS comes first, then a `0b0001` tag and the DTS.
        0b11 if header_data.len() >= 10
            && header_data[0] >> 4 == 0b0011
            && header_data[5] >> 4 == 0b0001
            && marker(header_data, 5, 0x01)
            && marker(header_data, 7, 0x01)
            && marker(header_data, 9, 0x01) =>
        {
            read_pts_secs(header_data)
        }
        // The header data does not have the shape its flags promise (`0b01` is not a
        // legal flag combination either). `vobsub`'s parser fails here, which makes
        // it skip the pack; rejecting the whole file instead would lose the
        // subpictures that were already read.
        _ => return Ok(PackOutcome::Resync),
    };
    // A malformed timestamp is a parse failure for `vobsub` too, so skip the pack
    // rather than turning it into "subtitle without timing info".
    if pts_dts_flags != 0b00 && pts_secs.is_none() {
        return Ok(PackOutcome::Resync);
    }

    Ok(PackOutcome::Packet(
        PesPayload {
            pts_secs,
            substream_id,
            data: &body[3 + header_data_len + 1..],
        },
        after,
    ))
}

/// Timing information recovered from one subpicture packet.
struct RawTiming {
    start: f64,
    end: Option<f64>,
}

/// Walks the control sequences of one assembled subpicture packet.
fn parse_control(raw: &[u8], base_time: f64) -> Result<RawTiming, Error> {
    // Two bytes of packet size, then two bytes holding the control block offset.
    if raw.len() < 4 {
        return Err(Error::UnexpectedEndOfSubtitleData);
    }
    let initial_control_offset = usize::from(be_u16(&raw[2..4]));

    let (mut start_time, mut end_time) = (None, None);
    let (mut coordinates, mut palette, mut alpha, mut rle_offsets) = (None, None, None, None);

    let mut control_offset = initial_control_offset;
    loop {
        if control_offset >= raw.len() {
            return Err(Error::ControlOffsetOutOfBounds {
                offset: control_offset,
                len: raw.len(),
            });
        }
        let seq = &raw[control_offset..];
        if seq.len() < 4 {
            return Err(Error::IncompleteControlSequence);
        }
        let date = be_u16(&seq[0..2]);
        let next = usize::from(be_u16(&seq[2..4]));
        let time = base_time + f64::from(date) / 100.0;

        let mut pos = 4usize;
        loop {
            let command = *seq.get(pos).ok_or(Error::IncompleteControlSequence)?;
            pos += 1;
            let operand_len = match command {
                0xff => break,
                // "force display", which does not affect the timing
                0x00 => 0,
                0x01 => {
                    start_time = start_time.or(Some(time));
                    0
                }
                0x02 => {
                    end_time = end_time.or(Some(time));
                    0
                }
                0x03 => {
                    let o = seq.get(pos..pos + 2).ok_or(Error::IncompleteControlSequence)?;
                    palette = palette.or(Some([o[0] >> 4, o[0] & 0xf, o[1] >> 4, o[1] & 0xf]));
                    2
                }
                0x04 => {
                    let o = seq.get(pos..pos + 2).ok_or(Error::IncompleteControlSequence)?;
                    alpha = alpha.or(Some([o[0] >> 4, o[0] & 0xf, o[1] >> 4, o[1] & 0xf]));
                    2
                }
                0x05 => {
                    let o = seq.get(pos..pos + 6).ok_or(Error::IncompleteControlSequence)?;
                    let x1 = u16::from(o[0]) << 4 | u16::from(o[1] >> 4);
                    let x2 = u16::from(o[1] & 0xf) << 8 | u16::from(o[2]);
                    let y1 = u16::from(o[3]) << 4 | u16::from(o[4] >> 4);
                    let y2 = u16::from(o[4] & 0xf) << 8 | u16::from(o[5]);
                    if x2 <= x1 || y2 <= y1 {
                        return Err(Error::InvalidBoundingBox);
                    }
                    coordinates = coordinates.or(Some((x1, y1, x2, y2)));
                    6
                }
                0x06 => {
                    let o = seq.get(pos..pos + 4).ok_or(Error::IncompleteControlSequence)?;
                    rle_offsets = Some([be_u16(&o[0..2]), be_u16(&o[2..4])]);
                    4
                }
                // Unknown command: `vobsub` swallows everything up to the next
                // 0xff terminator and logs it.
                _ => seq[pos..]
                    .iter()
                    .position(|&b| b == 0xff)
                    .ok_or(Error::IncompleteControlSequence)?,
            };
            pos += operand_len;
        }

        if next == control_offset {
            break; // points at itself: this was the last control sequence
        } else if next < control_offset {
            return Err(Error::ControlOffsetWentBackwards);
        }
        control_offset = next;
    }

    let start = start_time.ok_or(Error::MissingStartTime)?;

    // These four are unused here, but a packet missing any of them is rejected by
    // `vobsub` too, so keep rejecting it and keep the error wording.
    coordinates.ok_or(Error::MissingCoordinates)?;
    palette.ok_or(Error::MissingPalette)?;
    alpha.ok_or(Error::MissingAlpha)?;
    let rle_offsets = rle_offsets.ok_or(Error::MissingRleOffsets)?;

    // The same sanity check `vobsub` runs before decompressing, so structurally
    // broken files still fail. The `end > raw.len()` half is new: `vobsub` would
    // slice out of bounds and panic there.
    let start_0 = usize::from(rle_offsets[0]);
    let start_1 = usize::from(rle_offsets[1]);
    let end = initial_control_offset + 2;
    if start_0 > start_1 || start_1 > end || end > raw.len() {
        return Err(Error::InvalidScanLineOffsets);
    }
    // `vobsub 0.2.3` would now RLE-decode the bitmap. We only want the timings, so
    // we stop here.

    Ok(RawTiming { start, end: end_time })
}

/// Extracts every subpicture timing from a `.sub` byte stream.
pub fn timings(input: &[u8]) -> Result<Vec<VobSubTiming>, Error> {
    let mut packets = PesPackets { remaining: input };
    let mut raw: Vec<RawTiming> = Vec::new();

    'outer: loop {
        let first = match packets.next() {
            None => break,
            Some(Err(e)) => return Err(e),
            Some(Ok(p)) => p,
        };
        let base_time = first.pts_secs.ok_or(Error::SubtitleWithoutTiming)?;
        if first.data.len() < 2 {
            return Err(Error::PacketTooShort);
        }
        let wanted = usize::from(be_u16(&first.data[0..2]));
        let substream_id = first.substream_id;
        let mut buf = first.data.to_vec();

        // A subpicture may be split over several PES packets.
        while buf.len() < wanted {
            match packets.next() {
                // `vobsub` ends its iterator here, silently dropping the
                // half-assembled subtitle. Same behaviour.
                None => break 'outer,
                Some(Err(e)) => return Err(e),
                Some(Ok(next)) => {
                    if next.substream_id != substream_id {
                        continue;
                    }
                    buf.extend_from_slice(next.data);
                }
            }
        }
        buf.truncate(wanted.min(buf.len()));
        raw.push(parse_control(&buf, base_time)?);
    }

    // End-time fixup, the rule from `vobsub::Subtitles`.
    let mut out = Vec::with_capacity(raw.len());
    for (i, timing) in raw.iter().enumerate() {
        let start = timing.start;
        let end = match timing.end {
            Some(end) => end,
            None => match raw.get(i + 1) {
                Some(next) => (next.start - DEFAULT_SUBTITLE_SPACING).min(start + DEFAULT_SUBTITLE_LENGTH),
                None => start + DEFAULT_SUBTITLE_LENGTH,
            },
        };
        out.push(VobSubTiming {
            start_secs: start,
            end_secs: end,
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `vobsub 0.2.3` on `example.sub`, to four decimal places.
    #[test]
    fn example_sub_matches_vobsub() {
        let data = include_bytes!("../../fixtures/example.sub");
        let timings = timings(data).unwrap();
        assert_eq!(timings.len(), 2);
        assert_eq!(round4(&timings), [(49.4661, 50.9661), (52.6359, 55.5659)]);
    }

    /// A one-subtitle file written by Subtitle Edit.
    #[test]
    fn tiny_sub_matches_vobsub() {
        let data = include_bytes!("../../fixtures/tiny.sub");
        let timings = timings(data).unwrap();
        assert_eq!(round4(&timings), [(1.0, 2.74)]);
    }

    /// The same subtitle, but split across two PES packets.
    #[test]
    fn tiny_split_sub_matches_vobsub() {
        let data = include_bytes!("../../fixtures/tiny-split.sub");
        let timings = timings(data).unwrap();
        assert_eq!(round4(&timings), [(1.0, 2.74)]);
        assert_eq!(timings, timings_of("tiny"));
    }

    /// Truncating the stream drops the half-assembled subtitle instead of failing.
    #[test]
    fn truncated_stream_keeps_the_complete_subtitles() {
        let data = include_bytes!("../../fixtures/example.sub");
        let timings = timings(&data[..data.len() / 2]).unwrap();
        assert_eq!(round4(&timings), [(49.4661, 50.9661)]);
    }

    #[test]
    fn empty_and_garbage_input_yield_no_timings() {
        assert_eq!(timings(&[]).unwrap(), []);
        assert_eq!(timings(b"not an mpeg program stream at all").unwrap(), []);
    }

    /// A pack header that is cut short is reported, not panicked on.
    #[test]
    fn truncated_pack_header_is_an_error() {
        assert_eq!(timings(&PS_PACK_START), Err(Error::IncompletePesPacket));
    }

    /// A start code followed by a non-MPEG-2 header resynchronises past it without
    /// slicing out of bounds.
    #[test]
    fn unparsable_pack_resynchronises() {
        let mut data = PS_PACK_START.to_vec();
        data.extend_from_slice(&[0x00; 10]); // version bits are 0b00, not 0b01
        assert_eq!(timings(&data).unwrap(), []);
    }

    /// A `.sub` file that ends with a short pack which is *not* a subtitle pack must
    /// still yield the subtitles that came before it - `vobsub 0.2.3` resynchronises
    /// past such a pack. Testing the packet length before the start code turned this
    /// into `IncompletePesPacket` and threw the whole file away.
    #[test]
    fn a_short_trailing_non_subtitle_pack_does_not_discard_the_file() {
        let tiny: &[u8] = include_bytes!("../../fixtures/tiny.sub");
        let pack_header = &tiny[..14];

        for tail in [
            &[0x00, 0x00, 0x01, 0xe0, 0x12][..], // truncated video PES packet
            &[0x00, 0x00, 0x01, 0xe0][..],       // ...cut even shorter
            &[0xaa, 0xbb, 0xcc][..],             // not a start code at all
        ] {
            let mut data = tiny.to_vec();
            data.extend_from_slice(pack_header);
            data.extend_from_slice(tail);
            assert_eq!(round4(&timings(&data).unwrap()), [(1.0, 2.74)], "tail {tail:02x?}");
        }

        // ...but a *subtitle* pack cut off inside its length field is still a
        // truncated stream, exactly as in `vobsub`.
        let mut truncated = pack_header.to_vec();
        truncated.extend_from_slice(&[0x00, 0x00, 0x01, 0xbd, 0x00]);
        assert_eq!(timings(&truncated), Err(Error::IncompletePesPacket));
    }

    /// Four bytes that happen to look like a pack start code at the very end of a
    /// file must not cost us the subtitles before them: `vobsub` checks the MPEG-2
    /// version tag first and resynchronises when it does not match.
    #[test]
    fn trailing_garbage_after_a_start_code_does_not_discard_the_file() {
        let tiny: &[u8] = include_bytes!("../../fixtures/tiny.sub");
        // The version tag does not say MPEG-2, so the pack is skipped.
        for tail in [&[0xcf][..], &[0xff, 0x00, 0x00][..]] {
            let mut data = tiny.to_vec();
            data.extend_from_slice(&PS_PACK_START);
            data.extend_from_slice(tail);
            assert_eq!(round4(&timings(&data).unwrap()), [(1.0, 2.74)], "tail {tail:02x?}");
        }

        // A bare start code, or one followed by a genuinely truncated MPEG-2 pack
        // header, is a truncated stream in `vobsub` as well.
        for tail in [&[][..], &[0x44, 0x02, 0xc4, 0x82][..]] {
            let mut data = tiny.to_vec();
            data.extend_from_slice(&PS_PACK_START);
            data.extend_from_slice(tail);
            assert_eq!(timings(&data), Err(Error::IncompletePesPacket), "tail {tail:02x?}");
        }
    }

    /// The marker bits in the pack header are `tag_bits!` in `vobsub`, so a pack that
    /// gets them wrong is skipped rather than accepted with a corrupt clock - and
    /// accepting it can make the *next* stage reject the whole file.
    #[test]
    fn a_pack_header_with_bad_marker_bits_is_skipped() {
        let tiny: &[u8] = include_bytes!("../../fixtures/tiny.sub");
        // byte 4+0 bit 0x04, 4+2 bit 0x04, 4+4 bit 0x04, 4+5 bit 0x01 and 4+8 bits
        // 0x03 all have to be set.
        for (offset, mask) in [(4usize, 0x04u8), (6, 0x04), (8, 0x04), (9, 0x01), (12, 0x03)] {
            let mut data = tiny.to_vec();
            data[offset] &= !mask;
            assert_eq!(timings(&data).unwrap(), [], "byte {offset} without {mask:#04x}");
        }
        // ...and with them intact the fixture still parses.
        assert_eq!(round4(&timings(tiny).unwrap()), [(1.0, 2.74)]);
    }

    /// PES header data that does not have the shape its flags promise makes `vobsub`
    /// skip the pack, not reject the file.
    #[test]
    fn a_pack_with_unparsable_pes_header_data_is_skipped() {
        let tiny: &[u8] = include_bytes!("../../fixtures/tiny.sub");
        // byte 21 is the flag byte: 0b01 is not a legal combination, and 0b11
        // promises a PTS+DTS pair that is not there.
        // byte 23 is the first header-data byte, whose top nibble tags the PTS.
        for (offset, value) in [(21usize, 0x40u8), (21, 0xc0), (23, 0x57)] {
            let mut data = tiny.to_vec();
            data[offset] = value;
            assert_eq!(timings(&data).unwrap(), [], "byte {offset} = {value:02x}");
        }

        // Flags `0b00` really do mean "no timestamp", and a subpicture that starts in
        // such a packet is still rejected - with `vobsub`'s wording.
        let mut data = tiny.to_vec();
        data[21] = 0x00;
        assert_eq!(timings(&data), Err(Error::SubtitleWithoutTiming));
    }

    fn round4(timings: &[VobSubTiming]) -> Vec<(f64, f64)> {
        timings
            .iter()
            .map(|t| {
                (
                    (t.start_secs * 10000.0).round() / 10000.0,
                    (t.end_secs * 10000.0).round() / 10000.0,
                )
            })
            .collect()
    }

    fn timings_of(name: &str) -> Vec<VobSubTiming> {
        let data: &[u8] = match name {
            "tiny" => include_bytes!("../../fixtures/tiny.sub"),
            other => panic!("unknown fixture {other}"),
        };
        timings(data).unwrap()
    }
}
