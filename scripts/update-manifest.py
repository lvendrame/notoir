#!/usr/bin/env python3
"""Writes update.json (docs/specs/31 §Updates) for a release.

Usage: update-manifest.py <dist dir> <version> <download base URL>

Lists the installer the updater uses per platform — the universal .dmg
(macos), the .msi (windows) and the AppImage (linux-appimage) — with its
URL (<base>/<file name>) and SHA-256. The release workflow then signs the
file with minisign (update.json.minisig); Notoir offers nothing unless that
signature verifies against the public key built into it.
"""
import hashlib
import json
import pathlib
import sys

PATTERNS = {
    "macos": "*.dmg",
    "windows": "*.msi",
    "linux-appimage": "*.AppImage",
}


def sha256(path: pathlib.Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def main() -> int:
    if len(sys.argv) != 4:
        print(__doc__, file=sys.stderr)
        return 2
    dist, version, base = pathlib.Path(sys.argv[1]), sys.argv[2].lstrip("v"), sys.argv[3].rstrip("/")
    artifacts = {}
    for key, pattern in PATTERNS.items():
        found = sorted(dist.glob(pattern))
        if found:
            f = found[0]
            artifacts[key] = {"url": f"{base}/{f.name}", "sha256": sha256(f)}
    if not artifacts:
        print("no installers found", file=sys.stderr)
        return 1
    manifest = {"version": version, "artifacts": artifacts}
    (dist / "update.json").write_text(json.dumps(manifest, indent=2) + "\n")
    print(json.dumps(manifest, indent=2))
    return 0


if __name__ == "__main__":
    sys.exit(main())
