# Media dependencies and packaging

Images/GIFs use image 0.25.10, already present in Notoir. Its streaming GIF iterator composites
frames and exposes loop metadata. ffmpeg-next 9.1.0 handles audio/video demuxing and decoding;
rodio 0.23.0 provides native sound output with its default decoder/recording features disabled.
FFmpeg decodes audio too, avoiding duplicate audio decoder stacks. These are the current stable
crate versions verified for this change. egui-video 0.9.0 uses egui 0.29 and is incompatible
with the application's egui 0.36.2. GStreamer would add a larger plugin/runtime distribution.
alhazen-core's native-codec coverage varies with OS/hardware; bundled FFmpeg gives a consistent
baseline on Windows, Intel/Apple Silicon macOS and Linux.

## Building

For development, install compatible FFmpeg development libraries (FFmpeg 7 through 9.1,
with pkg-config and libclang). Linux sound output also needs ALSA development headers
(`libasound2-dev`). This does not change installer requirements: release packages bundle FFmpeg.

The reproducible release runtime is built from FFmpeg 9.1 with a source SHA256 pinned in
`scripts/build-media-runtime.sh`. Only local-file protocols, supported demuxers/decoders,
resampling and scaling are enabled; no encoders, networking, GPL, nonfree or optional external
codec libraries. H.264, HEVC, VP8/VP9 and the documented audio formats form the bundled baseline.
AV1 and DRM are outside that baseline; OS-installed FFmpeg may support extra codecs in development.

```sh
bash scripts/build-media-runtime.sh
source target/native/media-env.sh
cargo test --workspace
```

A C compiler, make, tar, xz, curl and pkg-config are required. x86 builds need NASM.
On Windows, run through MSYS2 MINGW64 with an MSVC developer environment on PATH;
pass `x86_64-pc-windows-msvc`. CI's local `media-runtime` action provisions these tools.
`FFMPEG_SOURCE_ARCHIVE` can point to an already downloaded source tarball; its checksum is
still verified. Build artifacts remain under `target/native` and are cached by script hash.

`package.sh` builds each macOS architecture separately, creates universal dylibs, and
places them in the app Resources/media directory with a relative executable rpath.
Debian/AppImage place libraries in `usr/lib/notoir/media` with relative rpaths.
Windows installers and MSIX place DLLs beside the executable. The generated packager config
comes from `scripts/prepare-media-package.py`; it refuses a missing native payload.
Linux packaging additionally requires patchelf. Python 3.11+ generates packaging metadata.
Store packaging requires FFMPEG_DIR from the CI action and bundles the same native DLLs.

Every package set includes the exact unmodified FFmpeg source archive and build script.
FFmpeg's LGPL license/build configuration are installed with its libraries. About credits FFmpeg.
Publish the matching source/build artifacts alongside installer downloads and retain the existing
release verification gates. Cargo-deny exceptions are scoped to the two WTFPL Rust wrapper crates;
FFmpeg itself is built as dynamically linked LGPL-2.1-or-later libraries.
See https://ffmpeg.org/legal.html for the upstream distribution requirements.

## Validation

Decoder tests exercise actual PNG/GIF, WebM and PCM WAV decoding. Audio tests use a deterministic
null sink, never the developer's speakers. UI tests verify media identity, save protection, split
views, zoom, session restoration and pause behavior. Render snapshots cover all built-in themes
at normal/narrow widths. Application-managed memory is checked through explicit reservations;
FFmpeg's internal codec allocations are additional. Profile long/large real files and repeated
open/close, and test actual audio output, seek accuracy and A/V synchronization on each platform.

Before release, test every installer on a clean machine without separately installed FFmpeg.
Verify playback with the bundled baseline codecs, dependencies/rpaths, MSIX activation,
physical volume/mute, device removal and frame/worker cleanup after closing tabs.
