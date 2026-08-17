# alass-subparse

Loads, changes and writes the common subtitle formats: `.srt` (SubRip), `.ssa`/`.ass`
(SubStation Alpha), `.idx` (VobSub index), `.sub` (MicroDVD and binary VobSub).

This is a vendored fork of [`subparse` 0.7.0](https://github.com/kaegi/subparse) by kaegi,
which has been unmaintained since 2019. It is kept in this workspace instead of as a
crates.io dependency because the published version pins a dependency stack that no longer
has a future - most pressingly `vobsub 0.2.3`, which hard-caps `nom` below 2.2 and so
drags in `nom 2.1.0`, the one crate in the tree that Rust has announced will stop
compiling ("trailing semicolon in macro used in expression position", rust-lang/rust#79813).

The original source is licensed under the MPL-2.0 (see `LICENSE`); the file headers are
kept intact. `src/formats/vobsub_timings.rs` is derived from `vobsub 0.2.3`
(CC0-1.0, Eric Kidd, <https://github.com/emk/subtitles-rs>), as are the three `.sub`
files in `fixtures/`.

## Dependencies

`chardetng`, `encoding_rs`, `thiserror` - 47 packages became 10. Gone: `failure`,
`combine`, `itertools`, `chardet` (the tree's only LGPL crate), `vobsub`, `nom 2.1`,
`image 0.13`, `regex 0.2`, `error-chain`, `backtrace`, and everything they pulled in.

## Changes from `subparse 0.7.0`

Structural:

- `failure` -> `thiserror 2`; every error type now implements `std::error::Error` with a
  working `source()` chain, which is what unblocks `anyhow` in `alass-cli`
- `combine` parser combinators -> hand-written line scanners (all eleven parser sites
  were fixed-shape scans over a single line)
- `chardet` -> `chardetng` for charset detection, preceded by an explicit BOM sniff
- the `vobsub` crate -> an in-tree timing-only reader (`formats/vobsub_timings.rs`); the
  RLE bitmap decoding `vobsub` did was thrown away unread on every file
- `itertools` -> std iterators; edition 2015 -> 2024; `#![forbid(unsafe_code)]`

Bug fixes, each covered by a test:

- **`.idx` writing panicked on every file.** `update_subtitle_entries` indexed
  `ts[count - 1]` with `count` starting at `0`, so `alass ref.srt in.idx out.idx` aborted
  with an index underflow.
- **`SubAssign` added instead of subtracting**, so `-=` moved `TimeDelta`, `TimePoint`
  and `TimeSpan` in the wrong direction.
- **`.ssa`/`.ass` line endings were rewritten.** Every non-`Dialogue:` line was written
  back with `\n`, so a CRLF file came back with mixed line endings and a file without a
  trailing newline gained one - in a parser that advertises non-destructive round-tripping.
- **`windows-1251` text was mis-decoded** as `x-mac-cyrillic` and written back out as
  UTF-8 mojibake, because every writer emits UTF-8 unconditionally.
- **A long digit run panicked.** `"99999999999999999999:0:0,0"` in a corrupt file
  overflowed an unwrapped `i64` parse; it is now a parse error.
- **`MicroDVD` output was not deterministic.** Formatting tags were emitted in `HashSet`
  iteration order, so a line with two or more of them serialized differently on each run.
  They are now sorted.
- **A `.ssa` `Format:` line with leading whitespace panicked** on an `assert!` in
  `SsaFieldsInfo::new_from_fields_info_line`.
- **A bad `.ssa` timepoint reported the wrong thing:** the "the timepoint `...` has wrong
  format" message was filled with a rendered parser error instead of the offending text.
- The four `assert_eq!`s guarding entry counts in `update_subtitle_entries` are now a
  returned `Error::EntryCountMismatch` rather than a panic on a public API.
- The `ParseIntError` behind `expected SubRip index line` is kept as a `source()`, so
  `alass-cli` still prints the fourth `caused by: invalid digit found in string` line
  that `failure` used to produce.

- **A truncated or padded binary `.sub` was rejected whole.** The timing reader checked
  the PES packet length before the PES start code, and the pack header's length before
  its MPEG-2 version tag, so a short trailing pack that is *not* a subtitle - four bytes
  of trailing garbage, a cut-off video packet - reported `IncompletePesPacket` and threw
  away every subtitle already read. `vobsub` resynchronises past such a pack. Likewise,
  PES header data that does not match the shape its flags promise now skips the pack
  instead of failing the file, and the pack-header marker bits are checked as `vobsub`
  checks them.

Deliberate behaviour differences:

- `chardetng` guesses a different encoding from `chardet` on some inputs. Over 114
  generated files (14 languages x up to 3 legacy encodings x 3 lengths) it was right
  where `chardet` was wrong 12 times - Polish `windows-1250`/`ISO-8859-2`, Turkish
  `windows-1254`/`ISO-8859-9`, and short UTF-8 files - and never wrong where `chardet`
  was right. The one construction found where it does worse is a Western-European file
  whose *only* non-ASCII byte is `0xEE`: `chardetng` reads that as Baltic
  (`windows-1257`, so `î` becomes `ī`). Adding any second accented letter makes it agree
  again. `chardetng` also never guesses UTF-16, which is now reached through a BOM sniff
  instead; UTF-16 without a BOM fails to parse, as it did before.
- A `.sub` whose RLE bitmap is corrupt now parses successfully and yields its timings;
  `vobsub` rejected the whole file. Only the timings were ever used. This is the only
  difference left over 30,549 differential cases (the `vobsub` fixtures, every prefix of
  them, and ~12,000 mutations): no input produces a different timing, a lost timing, a
  new failure or a panic.
- A scan-line offset past the end of a packet is reported as an error where `vobsub`
  would slice out of bounds and panic.

The public API is otherwise unchanged, so it can be diffed against the published crate:
the same 43-file corpus produces byte-identical output from both, apart from the two
`.idx` files that used to panic.
