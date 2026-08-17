#!/usr/bin/env bash
#
# End-to-end smoke test for `alass-cli`.
#
# Builds two subtitle files where the second one is the first one shifted by a
# known offset, lets `alass` re-align it and checks that the offset was
# recovered. The same check is repeated against a synthetic video file when
# `ffmpeg`/`ffprobe` are available, which exercises the audio extraction path.
#
# Usage:
#   scripts/smoke-test.sh [path-to-alass-binary]
#
# Environment:
#   ALASS_REQUIRE_VIDEO=1   fail (instead of skip) when ffmpeg is missing
#   ALASS_FFMPEG_PATH       ffmpeg binary used by alass and by this script
#   ALASS_FFPROBE_PATH      ffprobe binary used by alass and by this script

set -euo pipefail

ALASS_BIN="${1:-target/release/alass-cli}"

if [ ! -x "$ALASS_BIN" ]; then
	echo "error: '$ALASS_BIN' is not an executable file" >&2
	exit 1
fi

# `alass` is run from a temporary directory, so the path has to be absolute
case "$ALASS_BIN" in
/*) ;;
*) ALASS_BIN="$PWD/$ALASS_BIN" ;;
esac

FFMPEG="${ALASS_FFMPEG_PATH:-ffmpeg}"
FFPROBE="${ALASS_FFPROBE_PATH:-ffprobe}"

WORK_DIR="$(mktemp -d)"
trap 'rm -rf "$WORK_DIR"' EXIT

# start of the first subtitle in `reference.srt`, in milliseconds
EXPECTED_START_MS=10000

# the offset `incorrect.srt` is shifted by
OFFSET_MS=7500

cat >"$WORK_DIR/reference.srt" <<'EOF'
1
00:00:10,000 --> 00:00:12,000
Hello world

2
00:00:15,500 --> 00:00:17,500
Second line

3
00:00:30,000 --> 00:00:33,000
Third line

4
00:00:45,250 --> 00:00:47,000
Fourth line

5
00:01:02,000 --> 00:01:05,000
Fifth line
EOF

cat >"$WORK_DIR/incorrect.srt" <<'EOF'
1
00:00:17,500 --> 00:00:19,500
Hello world

2
00:00:23,000 --> 00:00:25,000
Second line

3
00:00:37,500 --> 00:00:40,500
Third line

4
00:00:52,750 --> 00:00:54,500
Fourth line

5
00:01:09,500 --> 00:01:12,500
Fifth line
EOF

# `alass`/`ffmpeg` are native Windows binaries when this runs in a Git Bash/MSYS
# shell, so file arguments have to be handed over as Windows paths
native_path() {
	if command -v cygpath >/dev/null 2>&1; then
		cygpath -w "$1"
	else
		printf '%s' "$1"
	fi
}

# prints the start timestamp of the first subtitle of an `.srt` file in milliseconds
first_start_ms() {
	awk -F' --> ' '/-->/ { split($1, t, /[:,]/); print (t[1] * 3600 + t[2] * 60 + t[3]) * 1000 + t[4]; exit }' "$1"
}

# assert_start <file> <tolerance in ms> <description>
assert_start() {
	local file="$1" tolerance="$2" description="$3"
	local actual diff

	if [ ! -s "$file" ]; then
		echo "FAIL: $description - no output was written" >&2
		exit 1
	fi

	actual="$(first_start_ms "$file")"
	diff=$((actual - EXPECTED_START_MS))
	[ "$diff" -lt 0 ] && diff=$((-diff))

	if [ "$diff" -gt "$tolerance" ]; then
		echo "FAIL: $description - first subtitle starts at ${actual}ms," \
			"expected ${EXPECTED_START_MS}ms (+-${tolerance}ms)" >&2
		exit 1
	fi

	echo "ok: $description (off by ${diff}ms)"
}

echo "=== subtitle reference (offset ${OFFSET_MS}ms) ==="
"$ALASS_BIN" --no-split --min-score 0.9 \
	"$(native_path "$WORK_DIR/reference.srt")" \
	"$(native_path "$WORK_DIR/incorrect.srt")" \
	"$(native_path "$WORK_DIR/subtitle-output.srt")"
assert_start "$WORK_DIR/subtitle-output.srt" 5 "aligned against a reference subtitle"

echo
echo "=== .idx output (offset ${OFFSET_MS}ms) ==="

# `.idx` writing used to abort the program, and it is the one format whose writer
# rebuilds the file around the timestamps instead of rewriting it wholesale. An `.idx`
# only stores start points - each subtitle runs until the next one - so this aligns an
# `.idx` against an `.idx` rather than against the `.srt` above.
idx_file() {
	cat >"$1" <<EOF
# VobSub index file, v7 (do not modify this line!)
size: 720x480
langidx: 0

id: en, index: 0
timestamp: $2, filepos: 000000000
timestamp: $3, filepos: 000001000
timestamp: $4, filepos: 000002000
EOF
}

idx_file "$WORK_DIR/reference.idx" 00:00:10:000 00:00:15:500 00:01:02:000
idx_file "$WORK_DIR/incorrect.idx" 00:00:17:500 00:00:23:000 00:01:09:500

"$ALASS_BIN" --no-split --min-score 0.9 \
	"$(native_path "$WORK_DIR/reference.idx")" \
	"$(native_path "$WORK_DIR/incorrect.idx")" \
	"$(native_path "$WORK_DIR/idx-output.idx")"

idx_start_ms="$(awk -F'[ ,]' '/^timestamp:/ { split($2, t, ":"); print (t[1] * 3600 + t[2] * 60 + t[3]) * 1000 + t[4]; exit }' "$WORK_DIR/idx-output.idx")"
idx_diff=$((idx_start_ms - EXPECTED_START_MS))
[ "$idx_diff" -lt 0 ] && idx_diff=$((-idx_diff))
if [ "$idx_diff" -gt 5 ]; then
	echo "FAIL: .idx output starts at ${idx_start_ms}ms, expected ${EXPECTED_START_MS}ms" >&2
	exit 1
fi
if ! grep -q 'filepos: 000002000' "$WORK_DIR/idx-output.idx"; then
	echo "FAIL: .idx output lost the fields around the timestamps" >&2
	exit 1
fi
echo "ok: rewrote an .idx file (off by ${idx_diff}ms)"

echo
echo "=== video reference (offset ${OFFSET_MS}ms) ==="

if ! command -v "$FFMPEG" >/dev/null 2>&1 || ! command -v "$FFPROBE" >/dev/null 2>&1; then
	if [ "${ALASS_REQUIRE_VIDEO:-0}" = "1" ]; then
		echo "FAIL: ffmpeg/ffprobe not found, but ALASS_REQUIRE_VIDEO=1" >&2
		exit 1
	fi
	echo "skipped: ffmpeg/ffprobe not found"
	echo
	echo "smoke test passed"
	exit 0
fi

# A black video with a noise burst wherever `reference.srt` has a subtitle; the
# voice activity detector picks the bursts up like speech.
"$FFMPEG" -y -loglevel error \
	-f lavfi -i "color=black:s=128x72:r=5:d=70" \
	-f lavfi -i "anoisesrc=d=70:c=pink:a=0.8:r=16000:seed=42" \
	-filter_complex "[1:a]volume=volume='between(t,10,12)+between(t,15.5,17.5)+between(t,30,33)+between(t,45.25,47)+between(t,62,65)':eval=frame[a]" \
	-map 0:v -map "[a]" -c:v mpeg4 -c:a aac -ac 1 -ar 16000 -shortest \
	"$(native_path "$WORK_DIR/reference.mp4")"

"$ALASS_BIN" --min-score 0.7 \
	"$(native_path "$WORK_DIR/reference.mp4")" \
	"$(native_path "$WORK_DIR/incorrect.srt")" \
	"$(native_path "$WORK_DIR/video-output.srt")"
assert_start "$WORK_DIR/video-output.srt" 500 "aligned against a video file"

echo
echo "smoke test passed"
