#!/usr/bin/env bash
# Builds Notoir release packages for the current OS into target/packages
# (docs/specs/31 §Packaging):
#   macOS   .app + .dmg (universal2: arm64 + x86_64 via lipo)
#   Windows .msi (WiX) + NSIS .exe
#   Linux   .deb + AppImage
# plus SHA256SUMS.txt and a debug-symbols archive. Signing runs only when the
# matching secrets are present in the environment:
#   macOS:   APPLE_SIGNING_IDENTITY (+ APPLE_ID, APPLE_TEAM_ID, APPLE_PASSWORD for notarization)
#   Windows: WINDOWS_CERTIFICATE_PATH + WINDOWS_CERTIFICATE_PASSWORD (signtool)
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

case "$(uname -s)" in
  Darwin)
    OS=macos
    rustup target add aarch64-apple-darwin x86_64-apple-darwin >/dev/null
    for t in aarch64-apple-darwin x86_64-apple-darwin; do
      cargo build --profile dist -p notoir --target "$t"
    done
    UNI=target/universal-apple-darwin/dist
    mkdir -p "$UNI"
    lipo -create -output "$UNI/notoir" \
      target/aarch64-apple-darwin/dist/notoir target/x86_64-apple-darwin/dist/notoir
    cargo packager -p notoir --profile dist --target universal-apple-darwin -f app
    APP="$OUT/Notoir.app"
    if [[ -n "${APPLE_SIGNING_IDENTITY:-}" ]]; then
      codesign --force --deep --options runtime --timestamp --sign "$APPLE_SIGNING_IDENTITY" "$APP"
    else
      echo "macOS signing skipped (no APPLE_SIGNING_IDENTITY): preview build"
    fi
    # create-dmg styles the window through Finder (AppleScript); without
    # automation access (headless machines) fall back to a plain image.
    if ! cargo packager -p notoir --profile dist --target universal-apple-darwin -f dmg; then
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
    cargo build --profile dist -p notoir
    cargo packager -p notoir --profile dist -f deb,appimage
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
    cargo build --profile dist -p notoir
    cargo packager -p notoir --profile dist -f wix,nsis
    if [[ -n "${WINDOWS_CERTIFICATE_PATH:-}" ]]; then
      for f in "$OUT"/*.msi "$OUT"/*-setup.exe; do
        signtool sign /f "$WINDOWS_CERTIFICATE_PATH" /p "$WINDOWS_CERTIFICATE_PASSWORD" /fd sha256 /tr http://timestamp.digicert.com /td sha256 "$f"
      done
    else
      echo "Authenticode signing skipped (no certificate): preview build"
    fi
    if [[ -f target/dist/notoir.pdb ]]; then
      (cd target/dist && 7z a "../packages/notoir-$VERSION-windows-x86_64-pdb.zip" notoir.pdb >/dev/null)
    fi
    ;;
  *) echo "unsupported OS: $(uname -s)"; exit 1 ;;
esac

# Checksums for everything shipped (apps are directories: only archives are summed).
(
  cd "$OUT"
  files=$(find . -maxdepth 1 -type f ! -name 'SHA256SUMS*' -printf '%f\n' 2>/dev/null || find . -maxdepth 1 -type f ! -name 'SHA256SUMS*' -exec basename {} \;)
  if command -v sha256sum >/dev/null; then sha256sum $files; else shasum -a 256 $files; fi > "SHA256SUMS-$OS.txt"
)
echo "packages in $OUT:"
ls -la "$OUT"
