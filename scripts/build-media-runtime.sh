#!/usr/bin/env bash
# Reproducible LGPL-only local-file decoder; no user-installed runtime needed.
# Run from the repo root. Exports are written to target/native/media-env.sh.
set -euo pipefail
cd "$(dirname "$0")/.."
repo=$PWD
version=9.1
sha256=1a4d394ae63d190a526cd052eb4415c156b5c5e2df37efab41a7de0b8fde2433
target=${1:-$(rustc -vV | sed -n 's/^host: //p')}
prefix="$repo/target/native/ffmpeg/$target"
source_archive=${FFMPEG_SOURCE_ARCHIVE:-"$repo/target/native/ffmpeg-$version.tar.xz"}
warning_patch="$repo/scripts/patches/ffmpeg-9.1-warning-fixes.patch"
mkdir -p "$repo/target/native"
if [[ ! -f "$source_archive" ]]; then
    curl --fail --location --retry 3 "https://ffmpeg.org/releases/ffmpeg-$version.tar.xz" -o "$source_archive"
fi
if command -v sha256sum >/dev/null; then actual=$(sha256sum "$source_archive"); else actual=$(shasum -a 256 "$source_archive"); fi
[[ ${actual%% *} == "$sha256" ]] || { echo 'FFmpeg source checksum mismatch' >&2; exit 1; }
# This script's checksum also invalidates cached native builds when flags change.
if command -v sha256sum >/dev/null; then stamp=$(cat "$0" "$warning_patch" | sha256sum); else stamp=$(cat "$0" "$warning_patch" | shasum -a 256); fi
stamp=${stamp%% *}
if [[ ! -f "$prefix/notoir-build-id" ]] || [[ $(cat "$prefix/notoir-build-id") != "$stamp" ]]; then
    # A changed recipe must not reuse objects compiled with older flags.
    build=$(mktemp -d "$repo/target/native/build-$target.XXXXXX")
    tar -xf "$source_archive" -C "$build"
    cd "$build/ffmpeg-$version"
    patch --batch --forward -p1 < "$warning_patch"
    args=(--prefix="$prefix" --disable-autodetect --disable-everything --disable-programs --disable-doc --disable-avdevice --disable-avfilter
        --disable-network --disable-gpl --disable-nonfree --disable-static --enable-shared --fatal-warnings
        --enable-avcodec --enable-avformat --enable-avutil --enable-swscale --enable-swresample
        --enable-protocol=file --enable-demuxer=mov,matroska,mp3,wav,flac,ogg,aac
        --enable-parser=h264,hevc,vp8,vp9,aac,mpegaudio,flac,opus,vorbis
        --enable-decoder=h264,hevc,vp8,vp9,aac,aac_fixed,mp3,mp3float,flac,vorbis,opus,alac,pcm_u8,pcm_s16le,pcm_s16be,pcm_s24le,pcm_s24be,pcm_s32le,pcm_s32be,pcm_f32le,pcm_f32be,pcm_f64le,pcm_f64be)
    case "$target" in
        *apple-darwin)
            arch=${target%%-*}; [[ $arch != aarch64 ]] || arch=arm64
            args+=(--enable-cross-compile --target-os=darwin --arch="$arch" --cc=clang
                --extra-cflags="-arch $arch -mmacosx-version-min=11.0" --extra-ldflags="-arch $arch -mmacosx-version-min=11.0"
                --install-name-dir=@rpath)
            ;;
        *windows-msvc)
            # Start in an MSVC developer environment, through MSYS2 bash.
            args+=(--toolchain=msvc --target-os=win64 --arch=x86_64 --disable-debug
                --host-ld="$PWD/compat/windows/mslink")
            ;;
        *linux-gnu) args+=(--cc=clang --enable-pic) ;;
        *) echo "Unsupported media build target: $target" >&2; exit 1 ;;
    esac
    ./configure "${args[@]}"
    # Feature probes deliberately compile unsupported code. Escalate warnings
    # for the actual build only, so probes cannot silently disable required flags.
    case "$target" in
        *apple-darwin) printf '\nCFLAGS += -Werror\nLDFLAGS += -Wl,-fatal_warnings\n' >> ffbuild/config.mak ;;
        *windows-msvc) printf '\nCFLAGS += -WX\nLDFLAGS += -WX\nHOSTCFLAGS += -WX\nHOSTLDFLAGS += -WX\n' >> ffbuild/config.mak ;;
        *linux-gnu)
            printf '\n%s\n%s\n' 'CFLAGS += -Werror' "LDFLAGS += -Wl,-rpath,'\$\$ORIGIN' -Wl,--fatal-warnings" >> ffbuild/config.mak
            ;;
    esac
    make -j "${NOTOIR_BUILD_JOBS:-4}"
    make install
    printf '%s\n' "$stamp" > "$prefix/notoir-build-id"
    cp COPYING.LGPLv2.1 "$prefix/FFmpeg-LICENSE.txt"
    printf '%s\n' "FFmpeg $version; source SHA256 $sha256" "${args[@]}" > "$prefix/FFmpeg-BUILD.txt"
fi
cd "$repo"
# Dynamic loader test paths are development-only; installers use relative paths.
{
    printf 'export FFMPEG_DIR=%q\n' "$prefix"
    printf 'export PKG_CONFIG_PATH=%q\n' "$prefix/lib/pkgconfig${PKG_CONFIG_PATH:+:$PKG_CONFIG_PATH}"
    printf 'export LD_LIBRARY_PATH=%q\n' "$prefix/lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
    printf 'export DYLD_LIBRARY_PATH=%q\n' "$prefix/lib${DYLD_LIBRARY_PATH:+:$DYLD_LIBRARY_PATH}"
    printf 'export PATH=%q\n' "$prefix/bin:$PATH"
} > target/native/media-env.sh
# Release packaging publishes this unmodified source archive alongside binaries.
cp "$source_archive" "$repo/target/native/ffmpeg-source-$version.tar.xz"
cp "$warning_patch" "$repo/target/native/FFmpeg-warning-fixes.patch"
echo "Media runtime ready: $prefix"
