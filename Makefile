# Packaging for the platforms the `build` workflow ships.
#
# Windows is not here: it needs no packaging beyond `cargo build --release`, and the rules
# that used to bundle ffmpeg with it downloaded from a host that has been gone since 2020.

package_linux64:
	cargo build --release --target x86_64-unknown-linux-musl
	cp ./target/x86_64-unknown-linux-musl/release/alass-cli ./target/alass-linux64

clean_linux64:
	rm -f target/alass-linux64

# Linux (ARM64/aarch64): same statically linked musl build as `package_linux64`.
# Build it on an ARM64 machine (or inside an ARM64 container), `musl-gcc` and
# the Rust target `aarch64-unknown-linux-musl` have to be installed.
package_linux_arm64:
	cargo build --release --target aarch64-unknown-linux-musl
	cp ./target/aarch64-unknown-linux-musl/release/alass-cli ./target/alass-linux-arm64

clean_linux_arm64:
	rm -f target/alass-linux-arm64

# macOS (Apple Silicon)
#
# `ffmpeg`/`ffprobe` are not bundled: macOS has no redistributable build like
# the Windows one, and the binary picks up whatever is on `PATH` anyway (or the
# paths in ALASS_FFMPEG_PATH / ALASS_FFPROBE_PATH).
package_macos:
	cargo build --release --target aarch64-apple-darwin
	cp ./target/aarch64-apple-darwin/release/alass-cli ./target/alass-macos-arm64
	rm -rf target/alass-macos
	mkdir target/alass-macos
	cp ./target/alass-macos-arm64 ./target/alass-macos/alass
	cp LICENSE ./target/alass-macos/LICENSE
	printf 'alass for macOS (Apple Silicon)\n\nRun it as `./alass movie.mp4 incorrect.srt output.srt`.\n\n`ffmpeg` and `ffprobe` have to be installed to read video files:\n\n    brew install ffmpeg\n\nTheir paths can be overridden with the environment variables\nALASS_FFMPEG_PATH and ALASS_FFPROBE_PATH.\n\nThis binary is not notarized, so macOS quarantines it after download.\nRemove the quarantine flag once with:\n\n    xattr -d com.apple.quarantine alass\n' > target/alass-macos/README.txt
	( cd target; tar -czf alass-macos-arm64.tar.gz alass-macos )

clean_macos:
	rm -f target/alass-macos-arm64 target/alass-macos-arm64.tar.gz
	rm -rf target/alass-macos

# end-to-end check of a release build (also runs the video path if ffmpeg is installed)
smoke_test:
	./scripts/smoke-test.sh target/release/alass-cli

.PHONY: package_linux64 clean_linux64 package_linux_arm64 clean_linux_arm64 \
	package_macos clean_macos smoke_test
