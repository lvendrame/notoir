#!/usr/bin/env python3
"""Native macOS acceptance for one-process windows, restoration and zero-window reopening.

Run after `cargo build -p notoir`. Uses a disposable NOTOIR_HOME and the opt-in
UI test bridge. --performance records idle CPU samples with 1, 3 and 10 windows.
"""
import argparse
import json
import os
from pathlib import Path
import subprocess
import tempfile
import time


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", default="target/debug/notoir")
    parser.add_argument("--artifacts", default="target/live-windows")
    parser.add_argument("--performance", action="store_true")
    args = parser.parse_args()
    binary = str(Path(args.binary).resolve())
    artifacts = Path(args.artifacts).resolve()
    artifacts.mkdir(parents=True, exist_ok=True)
    home = Path(tempfile.mkdtemp(prefix="notoir-windows-native-"))
    (home / "config").mkdir()
    (home / "config/settings.toml").write_text('[update]\nmode="off"\n')
    first_file, second_file = home / "a.txt", home / "b.txt"
    first_file.write_text("window A\n")
    second_file.write_text("window B\n")
    env = dict(os.environ, NOTOIR_HOME=str(home), NOTOIR_LANG="en",
               NOTOIR_TEST_COMMANDS="1", NOTOIR_LOG="debug")
    results, samples = [], []
    sequence = 0
    log = (artifacts / "desktop.log").open("w")
    process = subprocess.Popen([binary, str(first_file)], env=env, stdout=log, stderr=log)

    def launch(*arguments):
        subprocess.run([binary, *arguments], env=env, check=True, timeout=15,
                       stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)

    def command(name, arguments=None):
        launch("--command", name, "--args", json.dumps(arguments))

    def snapshot():
        nonlocal sequence
        sequence += 1
        output = artifacts / f"state-{sequence:03}.json"
        output.unlink(missing_ok=True)
        command("test.ui", {"op": "snapshot", "output": str(output)})
        deadline = time.monotonic() + 12
        while time.monotonic() < deadline:
            if process.poll() is not None:
                raise AssertionError(f"GUI exited; see {artifacts / 'desktop.log'}")
            try:
                return json.loads(output.read_text())
            except (FileNotFoundError, json.JSONDecodeError):
                time.sleep(.1)
        raise AssertionError(f"Snapshot timed out; profile: {home}")

    def check(condition, name):
        assert condition, (name, str(home))
        results.append(name)
        print("PASS:", name, flush=True)

    def cpu_seconds():
        value = subprocess.check_output(["ps", "-p", str(process.pid), "-o", "time="], text=True).strip()
        return sum(float(part) * 60 ** index for index, part in enumerate(reversed(value.split(":"))))

    try:
        time.sleep(3)
        state = snapshot()
        check(state["pid"] == process.pid and len(state["windows"]) == 1,
              "initial native window uses host PID")
        first = state["window_id"]
        launch("--new-window", str(second_file))
        time.sleep(1)
        state = snapshot()
        check(len(state["windows"]) == 2 and state["pid"] == process.pid,
              "CLI new window shares host PID")
        second = state["window_id"]
        check(second != first and any(d["text"] == "window B\n" for d in state["documents"]),
              "second window has independent tabs")
        command("file.newWindow")
        time.sleep(.7)
        state = snapshot()
        third = state["window_id"]
        check(len(state["windows"]) == 3 and not state["documents"],
              "UI New Window creates an empty context")
        command("window.activate", {"id": first})
        time.sleep(.7)
        state = snapshot()
        check(state["window_id"] == first and any(d["text"] == "window A\n" for d in state["documents"]),
              "Window navigation switches editing context")
        command("window.close")
        time.sleep(1)
        state = snapshot()
        check(len(state["windows"]) == 2 and process.poll() is None,
              "closing first-created editor preserves siblings")
        command("app.quit")
        process.wait(timeout=15)
        check(process.returncode == 0, "application Quit exits cleanly")
        process = subprocess.Popen([binary], env=env, stdout=log, stderr=log)
        time.sleep(3)
        state = snapshot()
        check(len(state["windows"]) == 2 and {w["id"] for w in state["windows"]} == {second, third},
              "restart restores all open windows and identities")
        command("window.close")
        time.sleep(.6)
        command("window.close")
        time.sleep(1)
        check(process.poll() is None, "macOS keeps application alive with zero editor windows")
        launch("--new-window")
        time.sleep(1)
        check(len(snapshot()["windows"]) == 1, "new-window launch works after closing all editors")
        if args.performance:
            for count in (1, 3, 10):
                while len(snapshot()["windows"]) < count:
                    command("file.newWindow")
                    time.sleep(.3)
                time.sleep(1)
                before, started = cpu_seconds(), time.monotonic()
                time.sleep(2)
                elapsed = time.monotonic() - started
                cpu = cpu_seconds() - before
                samples.append({"windows": count, "wall_seconds": elapsed,
                                "cpu_seconds": cpu, "cpu_percent_one_core": 100 * cpu / elapsed})
                print("CPU:", samples[-1], flush=True)
        command("app.quit")
        process.wait(timeout=15)
    finally:
        if process.poll() is None:
            process.terminate()
            process.wait(timeout=10)
        log.close()
        (artifacts / "results.json").write_text(json.dumps(
            {"profile": str(home), "passed": results, "idle_cpu_samples": samples}, indent=2) + "\n")
    print("Artifacts:", artifacts)


if __name__ == "__main__":
    main()
