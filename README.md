# Introduction

`alass` is a command line tool to synchronize subtitles to movies.

It can automatically correct

 - constant offsets
 - splits due to advertisement breaks, directors cut, ...
 - different framerates

The alignment process is not only fast and
accurate, but also language-agnostic. This means
you can align subtitles to movies in different
languages.

`alass` stands for  "Automatic Language-Agnostic Subtitle Synchronization". The theory and algorithms
are documented in my [bachelor's thesis](documentation/thesis.pdf)
and summarized in my [bachelor's presentation](documentation/slides.pdf).


## Executables

Grab one from the [releases page](https://github.com/lyl2dora/alass/releases), or build it
yourself with the commands in [Building the release binaries](#building-the-release-binaries).
Releases are cut by running the `build` workflow with a tag in its `release_tag` input, so every
published binary comes from a run that passed the tests and the end-to-end smoke test on its own
platform.

| Platform | Artifact |
| --- | --- |
| Windows x86-64 | `alass-windows64.exe` |
| Linux x86-64 | `alass-linux64` (statically linked, runs on any distribution) |
| Linux ARM64 | `alass-linux-arm64` (likewise) |
| macOS Apple Silicon | `alass-macos-arm64.tar.gz` |

Upstream's [2019 releases](https://github.com/kaegi/alass/releases) predate everything in this
fork - including a fix for `.idx` output, which used to abort the program.

To read video files, `ffmpeg` and `ffprobe` have to be installed. You can change their paths with
the environment variables `ALASS_FFMPEG_PATH` (default `ffmpeg`) and `ALASS_FFPROBE_PATH`
(default `ffprobe`).

### On macOS

Extract the archive and install `ffmpeg`:

```bash
$ tar -xzf alass-macos-arm64.tar.gz
$ brew install ffmpeg # provides both `ffmpeg` and `ffprobe`
$ ./alass-macos/alass movie.mp4 incorrect_subtitle.srt output.srt
```

The binary is not signed or notarized, so macOS puts a downloaded archive in quarantine and
refuses to start it ("cannot be opened because the developer cannot be verified"). Removing
the quarantine flag once is enough:

```bash
$ xattr -d com.apple.quarantine ./alass-macos/alass
```

## Usage

The most basic command is:

```bash
$ alass movie.mp4 incorrect_subtitle.srt output.srt
```

You can also use `alass` to align the incorrect subtitle to a different subtitle:

```bash
$ alass reference_subtitle.ssa incorrect_subtitle.srt output.srt
```

You can additionally adjust how much the algorithm tries to avoid introducing or removing a break:

```bash
# split-penalty is a value between 0 and 1000 (default 7)
$ alass reference_subtitle.ssa incorrect_subtitle.srt output.srt --split-penalty 10
```

Values between 5 and 20 are the most useful. Anything above 20 misses some important splits and anything below 5 introduces many unnecessary splits.

If you only want to shift the subtitle, without introducing splits, you can use `--no-split`:

```bash
# synchronizing the subtitles in this mode is very fast
$ alass movie.mp4 incorrect_subtitle.srt output.srt --no-split
```

Currently supported are `.srt`, `.ssa`/`.ass`, `.idx` and `.sub` files (MicroDVD, and VobSub as a
reference). Every common video format is supported for the reference file.


## Performance and Results

The extraction of the audio from a video takes about 10 to 20 seconds. Computing the alignment usually takes between 5 and 10 seconds.

The alignment is usually perfect -
the percentage of "good subtitles" is about 88% to 98%, depending on how strict you classify a "good subtitle".
Downloading random subtitles
from `OpenSubtitles.org` had an error rate of about 50%
(sample size N=118).
Of all subtitle _lines_ (not subtitle files) in the tested database,
after synchronization

 - 50% were within 50ms of target position
 - 80% were within 100ms of target position
 - 90% were within 400ms of target position
 - 95% were within 800ms of target position

compared to a (possibly not perfect) reference subtitle.

## How to compile the binary

This fork is not published on crates.io - `cargo install alass-cli` would fetch the 2019
release instead, so build it from source:

```bash
$ git clone https://github.com/kaegi/alass
$ cd alass
$ cargo build --release
$ cargo run --release -- movie.mp4 input.srt output.srt
```

It needs [Rust](https://www.rust-lang.org/tools/install) 1.88 or newer (the workspace is on
edition 2024); `rust-toolchain.toml` points rustup at the stable channel. The voice activity
module is written in C, so a C compiler (`gcc` or `clang`) has to be available too.

To use `alass-cli` with video files, `ffmpeg` and `ffprobe` have to be installed. They are
used to extract the raw audio data. You can set the paths used by `alass` using the
environment variables `ALASS_FFMPEG_PATH` (default `ffmpeg`) and `ALASS_FFPROBE_PATH`
(default `ffprobe`).

### Building the release binaries

| Platform | Rust target | Command | Output |
| --- | --- | --- | --- |
| Windows x86-64 | `x86_64-pc-windows-msvc` | `cargo build --release` | `target/release/alass-cli.exe` |
| Linux x86-64 | `x86_64-unknown-linux-musl` | `make package_linux64` | `target/alass-linux64` |
| Linux ARM64 | `aarch64-unknown-linux-musl` | `make package_linux_arm64` | `target/alass-linux-arm64` |
| macOS Apple Silicon | `aarch64-apple-darwin` | `make package_macos` | `target/alass-macos-arm64`, `target/alass-macos-arm64.tar.gz` |

The Linux binaries are statically linked against musl, so they run on any distribution;
building them needs `musl-gcc` (`apt install musl-tools`) and the matching Rust target
(`rustup target add ...`). Both Linux targets are built natively, i.e. on a machine or
container of their own architecture.

The same four binaries are built by the `build` GitHub Actions workflow. It is triggered
manually ("Run workflow" in the *Actions* tab, optionally for a single platform), runs
`cargo fmt --check`, `cargo clippy -D warnings`, `cargo test` and the smoke test on every
platform, and uploads the binaries as workflow artifacts. Filling in its `release_tag` input
additionally publishes them, with a `SHA256SUMS`, as a GitHub Release under that tag - and
forces the smoke test on, whatever `run_smoke_test` says, so nothing is ever published that has
not aligned a subtitle on the platform it was built for.

### Smoke test

`scripts/smoke-test.sh` aligns a subtitle that was shifted by a known offset - once against a
reference subtitle and once against a video file generated with `ffmpeg` - and checks that the
offset was recovered:

```bash
$ cargo build --release
$ make smoke_test
```

### FFmpeg as a library

By default `alass` spawns the `ffmpeg` and `ffprobe` executables to read the audio out of a
video file, which is what the released binaries do and needs nothing at build time. It can
instead link against the FFmpeg libraries, which extracts the audio a few seconds faster:

```bash
$ cargo build --release -p alass-cli --no-default-features --features ffmpeg-library
```

That needs the FFmpeg development packages and `pkg-config` at compile time
(`brew install ffmpeg pkg-config`, or `libavcodec-dev libavformat-dev libavutil-dev
libavfilter-dev libavdevice-dev libswresample-dev libswscale-dev` on Debian/Ubuntu), so it
does not work for the statically linked musl builds. The `build` workflow can check this
feature on Linux and macOS - tick "ffmpeg_library" when starting it.


### Alias Setup

*For Linux and macOS users:* the binary is called `alass-cli` when built from source, so it is
worth putting it on your path under the shorter name. Add this to your `~/.bashrc` (or the setup
file of your favorite shell), pointing it at wherever you keep the binary:

```bash
export PATH="$PATH:$HOME/.local/bin"
alias alass="alass-cli"
```

## Folder structure

This `cargo` workspace contains three crates:

  - `alass-core` which provides the algorithm

    It is targeted at *developers* who want to use the same algorithm in their project.

  - `alass-cli` which is the official command line tool

    It is target at *end users* who want to correct their subtitles.

  - `alass-subparse` which reads and writes the subtitle files

    A vendored fork of [`subparse`](https://github.com/kaegi/subparse) 0.7.0 (MPL-2.0),
    modernized in place - see `alass-subparse/README.md` for what changed and why.

## Library Documentation

[Open README](./alass-core/README.md) from `alass-core`.

## Notes

This program was called `aligner` in the past. This made it nearly impossible to find on a search engine, so `alass` was chosen instead.