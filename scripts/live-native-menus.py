#!/usr/bin/env python3
"""Verify real AppKit menu tracking and actions in an isolated macOS debug app.

The probe runs only inside this test's Notoir process; no Accessibility permission
or global input injection is needed. Build the app with cargo build -p notoir first.
"""
import argparse
import json
import os
from pathlib import Path
import subprocess
import tempfile


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", default="target/debug/notoir")
    parser.add_argument("--artifacts", default="target/live-native-menus")
    args = parser.parse_args()
    root = Path(__file__).resolve().parent.parent
    artifacts = Path(args.artifacts).resolve()
    artifacts.mkdir(parents=True, exist_ok=True)
    library = artifacts / "menu-probe.dylib"
    subprocess.run(["clang", "-dynamiclib", "-fobjc-arc", "-fblocks", "-framework", "Cocoa",
                    str(root / "scripts/native-menu-probe.m"), "-o", str(library)], check=True)
    result = artifacts / "results.json"
    result.unlink(missing_ok=True)
    with tempfile.TemporaryDirectory(prefix="notoir-native-menu-") as temporary:
        home = Path(temporary)
        (home / "config").mkdir()
        (home / "config/settings.toml").write_text('[update]\nmode="off"\n')
        env = dict(os.environ, NOTOIR_HOME=str(home), NOTOIR_LANG="en",
                   NOTOIR_TEST_COMMANDS="1", NOTOIR_MENU_PROBE_OUTPUT=str(result),
                   DYLD_INSERT_LIBRARIES=str(library))
        with (artifacts / "desktop.log").open("w") as log:
            process = subprocess.Popen([str(Path(args.binary).resolve())], env=env, stdout=log, stderr=log)
            try:
                process.wait(timeout=30)
                assert process.returncode == 0, f"GUI exited with {process.returncode}; see {artifacts}"
            finally:
                if process.poll() is None:
                    process.terminate()
                    process.wait(timeout=10)
        report = json.loads(result.read_text())
        assert "error" not in report, report
        for key in ("native_items_stable_across_frames", "file_menu_tracks_with_editor",
                    "app_menu_tracks_with_editor", "native_new_window_action", "zero_windows",
                    "app_menu_tracks_without_windows", "quit_dispatched"):
            assert report.get(key) is True, report
            print("PASS:", key)
    print("Artifacts:", artifacts)


if __name__ == "__main__":
    main()
