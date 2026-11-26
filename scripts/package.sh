#!/usr/bin/env bash
# Builds Notoir release packages for the current OS into target/packages
# (docs/specs/31 §Packaging):
#   macOS   .app + .dmg (universal2: arm64 + x86_64 via lipo)
#   Windows .msi (WiX) + NSIS .exe
#   Linux   .deb + AppImage
# plus SHA256SUMS.txt and a debug-symbols archive. Signing runs only when the
# matching secrets are present in the environment:
#   macOS:   APPLE_SIGNING_IDENTITY (+ APPLE_ID, APPLE_TEAM_ID, APPLE_PASSWORD for notarization)
#   Windows: WINDOWS_SIGNING_PROVIDER=azure|pfx + WINDOWS_EXPECTED_PUBLISHER
#            (see docs/windows-signing.md). WINDOWS_SIGNING_REQUIRED=true
#            rejects unsigned output; it is set for published CI releases.
#   Linux:   GPG_KEY_ID (detached signature for the AppImage)
# Requires cargo-packager 0.11 (`cargo install cargo-packager --locked`).
set -euo pipefail

cd "$(dirname "$0")/.."
# `path+file:///…/apps/notoir#1.2.3` (no Python: Windows runners lack python3).
VERSION=$(cargo pkgid -p notoir)
VERSION=${VERSION##*[#@]}
OUT=target/packages
rm -rf "$OUT"
mkdir -p "$OUT"

PYTHON_CMD=python3
command -v "$PYTHON_CMD" >/dev/null || PYTHON_CMD=python
# Every config includes the exact native libraries used by the linker.
media_package_config() {
    "$PYTHON_CMD" scripts/prepare-media-package.py "$1" "$2" "$3"
}
case "$(uname -s)" in
  Darwin)
    OS=macos
    rustup target add aarch64-apple-darwin x86_64-apple-darwin >/dev/null
    for t in aarch64-apple-darwin x86_64-apple-darwin; do
      bash scripts/build-media-runtime.sh "$t"
      source target/native/media-env.sh
      cargo build --profile dist -p notoir --target "$t"
      install_name_tool -delete_rpath "$FFMPEG_DIR/lib" "target/$t/dist/notoir"
    done
    UNI=target/universal-apple-darwin/dist
    mkdir -p "$UNI"
    lipo -create -output "$UNI/notoir" \
      target/aarch64-apple-darwin/dist/notoir target/x86_64-apple-darwin/dist/notoir
    MEDIA_UNI="$PWD/target/native/ffmpeg/universal-apple-darwin"
    mkdir -p "$MEDIA_UNI/lib"
    for lib in target/native/ffmpeg/aarch64-apple-darwin/lib/*.dylib; do
      lipo -create "$lib" "target/native/ffmpeg/x86_64-apple-darwin/lib/$(basename "$lib")" -output "$MEDIA_UNI/lib/$(basename "$lib")"
    done
    cp target/native/ffmpeg/aarch64-apple-darwin/FFmpeg-*.txt "$MEDIA_UNI/"
    CONFIG=$(media_package_config macos "$UNI" "$MEDIA_UNI")
    cargo packager --config "$CONFIG" --target universal-apple-darwin -f app
    APP="$OUT/Notoir.app"
    if [[ -n "${APPLE_SIGNING_IDENTITY:-}" ]]; then
      for lib in "$APP"/Contents/Resources/media/*.dylib; do
        codesign --force --options runtime --timestamp --sign "$APPLE_SIGNING_IDENTITY" "$lib"
      done
      codesign --force --deep --options runtime --timestamp --sign "$APPLE_SIGNING_IDENTITY" "$APP"
    else
      echo "macOS signing skipped (no APPLE_SIGNING_IDENTITY): preview build"
    fi
    cp "$APP/Contents/MacOS/notoir" "$UNI/notoir"
    cp "$APP"/Contents/Resources/media/*.dylib target/native/media-payload/
    # create-dmg styles the window through Finder (AppleScript); without
    # automation access (headless machines) fall back to a plain image.
    if ! cargo packager --config "$CONFIG" --target universal-apple-darwin -f dmg; then
      echo "styled DMG failed; building a plain DMG"
      rm -f "$OUT"/rw.*.dmg "$OUT"/*.dmg
      STAGE=$(mktemp -d)
      cp -R "$APP" "$STAGE/"
      ln -s /Applications "$STAGE/Applications"
      hdiutil create -volname Notoir -srcfolder "$STAGE" -ov -format UDZO "$OUT/Notoir_${VERSION}_universal.dmg"
      rm -rf "$STAGE"
    fi
    DMG=$(ls "$OUT"/*.dmg)
    if [[ -n "${APPLE_SIGNING_IDENTITY:-}" ]]; then
      codesign --force --timestamp --sign "$APPLE_SIGNING_IDENTITY" "$DMG"
      if [[ -n "${APPLE_ID:-}" && -n "${APPLE_TEAM_ID:-}" && -n "${APPLE_PASSWORD:-}" ]]; then
        xcrun notarytool submit "$DMG" --apple-id "$APPLE_ID" --team-id "$APPLE_TEAM_ID" --password "$APPLE_PASSWORD" --wait
        xcrun stapler staple "$DMG"
      fi
    fi
    (cd target && ditto -c -k --keepParent aarch64-apple-darwin/dist/notoir.dSYM "packages/notoir-$VERSION-macos-arm64-dSYM.zip")
    (cd target && ditto -c -k --keepParent x86_64-apple-darwin/dist/notoir.dSYM "packages/notoir-$VERSION-macos-x86_64-dSYM.zip")
    ;;
  Linux)
    OS=linux
    bash scripts/build-media-runtime.sh
    source target/native/media-env.sh
    cargo build --profile dist -p notoir
    CONFIG=$(media_package_config linux target/dist "$FFMPEG_DIR")
    cargo packager --config "$CONFIG" -f deb,appimage
    if [[ -n "${GPG_KEY_ID:-}" ]]; then
      for f in "$OUT"/*.AppImage; do gpg --batch --yes --local-user "$GPG_KEY_ID" --detach-sign --armor "$f"; done
    else
      echo "AppImage signing skipped (no GPG_KEY_ID)"
    fi
    if [[ -f target/dist/notoir.dwp ]]; then
      tar -C target/dist -czf "$OUT/notoir-$VERSION-linux-x86_64-debug.tar.gz" notoir.dwp
    fi
    ;;
  MINGW*|MSYS*|CYGWIN*)
    OS=windows
    # Git Bash puts /usr/bin first, where GNU coreutils' `link` shadows the MSVC
    # linker that rustc resolves as `link.exe` from PATH inside a VS dev env.
    msvc_linker=$(cygpath -u "${NOTOIR_MSVC_LINKER:?run scripts/enable-msvc.ps1 first}")
    export PATH="$PWD/scripts:$(dirname "$msvc_linker"):$PATH"
    if [[ ! "$(command -v link.exe)" -ef "$msvc_linker" ]]; then
      echo "Wrong Windows linker: PATH resolves '$(command -v link.exe)'; expected '$msvc_linker'" >&2
      exit 1
    fi
    pwsh -NoProfile -NonInteractive -File scripts/sign-windows.ps1 -ValidateOnly
    rm -rf target/windows-signing-receipts
    mkdir -p target/windows-signing-receipts
    export WINDOWS_SIGNING_RECEIPTS
    WINDOWS_SIGNING_RECEIPTS=$(cygpath -w "$PWD/target/windows-signing-receipts")
    bash scripts/build-media-runtime.sh
    source target/native/media-env.sh
    cargo build --profile dist -p notoir
    CONFIG=$(media_package_config windows target/dist "$FFMPEG_DIR")
    # The packager's hook signs the application before embedding it, both
    # installers, and the NSIS uninstaller. Each hook verifies trust/timestamp.
    export WINDOWS_SIGNING_PHASE=wix
    cargo packager --config "$CONFIG" -f wix
    export WINDOWS_SIGNING_PHASE=nsis
    cargo packager --config "$CONFIG" -f nsis
    # NSIS !uninstfinalize does not enforce the hook's exit status. Fresh
    # receipts prove every hook completed, including the embedded uninstaller.
    pwsh -NoProfile -NonInteractive -File scripts/sign-windows.ps1 -VerifyPackage
    if [[ -f target/dist/notoir.pdb ]]; then
      (cd target/dist && 7z a "../packages/notoir-$VERSION-windows-x86_64-pdb.zip" notoir.pdb >/dev/null)
    fi
    ;;
  *) echo "unsupported OS: $(uname -s)"; exit 1 ;;
esac

# Matching source and native build recipes accompany every package set.
cp target/native/ffmpeg-source-9.1.tar.xz "$OUT/"
cp scripts/build-media-runtime.sh "$OUT/FFmpeg-build.sh"
cp scripts/patches/ffmpeg-9.1-warning-fixes.patch "$OUT/FFmpeg-warning-fixes.patch"
# Checksums for everything shipped (apps are directories: only archives are summed).
(
  cd "$OUT"
  files=$(find . -maxdepth 1 -type f ! -name 'SHA256SUMS*' -printf '%f\n' 2>/dev/null || find . -maxdepth 1 -type f ! -name 'SHA256SUMS*' -exec basename {} \;)
  if command -v sha256sum >/dev/null; then sha256sum $files; else shasum -a 256 $files; fi > "SHA256SUMS-$OS.txt"
)
echo "packages in $OUT:"
ls -la "$OUT"
