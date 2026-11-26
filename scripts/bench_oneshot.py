#!/usr/bin/env python3
"""Measure real one-shot toggles and Whisper inference with isolated desktop helpers.

Use a 16 kHz mono WAV (at least 250 ms) and release binaries built from each
revision. No microphone, clipboard, or notification service is touched.
"""

import argparse
import json
import os
from pathlib import Path
import signal
import statistics
import subprocess
import sys
import tempfile
import time


def helper():
    case = Path(os.environ["BENCH_CASE"])
    name = Path(sys.argv[0]).name
    if name == "pw-record":
        def stop(*_):
            Path(sys.argv[-1]).write_bytes(Path(os.environ["BENCH_WAV"]).read_bytes())
            sys.exit(0)

        signal.signal(signal.SIGINT, stop)
        (case / "recorder.tmp").write_text(str(time.monotonic_ns()))
        (case / "recorder.tmp").rename(case / "recorder")
        while True:
            signal.pause()
    elif name == "notify-send":
        if sys.argv[-1].startswith(("Recording.", "Transcribing...")):
            time.sleep(float(os.environ["BENCH_NOTIFY_DELAY"]))
        print(1)
    elif name == "wl-copy":
        (case / "transcript").write_bytes(sys.stdin.buffer.read())
        (case / "copied.tmp").write_text(str(time.monotonic_ns()))
        (case / "copied.tmp").rename(case / "copied")


def wait_for(path, process):
    deadline = time.monotonic() + 20
    while not path.exists():
        if process.poll() is not None or time.monotonic() >= deadline:
            raise RuntimeError(f"did not observe {path.name}; see {path.parent / 'log'}")
        time.sleep(0.001)


def trial(root, binary, wav, args):
    case = Path(tempfile.mkdtemp(dir=root))
    config = case / "config" / "vdora"
    config.mkdir(parents=True)
    language_cfg = f"language = {json.dumps(args.language)}\n" if args.language else ""
    (config / "config.toml").write_text(
        f"model_path = {json.dumps(str(args.model.resolve()))}\nautopaste = false\n{language_cfg}"
    )
    env = dict(os.environ, XDG_RUNTIME_DIR=str(case), XDG_CONFIG_HOME=str(case / "config"),
               WAYLAND_DISPLAY="isolated", PATH=f"{root / 'bin'}:{os.defpath}",
               BENCH_CASE=str(case), BENCH_WAV=str(wav.resolve()),
               BENCH_NOTIFY_DELAY=str(args.notify_delay), RUST_LOG="debug")
    command = [str(binary.resolve()), "--oneshot"]
    if args.no_notify:
        command.append("--no-notify")
    with (case / "log").open("w") as log:
        started = time.monotonic_ns()
        with subprocess.Popen(command, env=env, stdout=log, stderr=log,
                              start_new_session=True) as parent:
            try:
                wait_for(case / "recorder", parent)
                recorder = int((case / "recorder").read_text())
                state = case / "vdora" / "oneshot" / "state"
                while "phase=recording" not in state.read_text():
                    if parent.poll() is not None or time.monotonic_ns() - recorder > 5_000_000_000:
                        raise RuntimeError("recorder did not become toggleable")
                    time.sleep(0.001)
                time.sleep(max(0, args.record_seconds - (time.monotonic_ns() - recorder) / 1e9))
                stopped = time.monotonic_ns()
                subprocess.run(command, env=env, stdout=log, stderr=log, check=True, timeout=20)
                parent.wait(timeout=20)
                if parent.returncode != 0:
                    raise RuntimeError(f"one-shot failed; see {case / 'log'}")
                wait_for(case / "copied", parent)
            except BaseException:
                try:
                    os.killpg(parent.pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
                print((case / "log").read_text(), file=sys.stderr)
                raise
    copied = int((case / "copied").read_text())
    transcript = (case / "transcript").read_text()
    return (recorder - started) / 1e6, (copied - stopped) / 1e6, transcript


def main(args):
    binaries = [args.baseline, args.candidate]
    with tempfile.TemporaryDirectory(prefix="vdora-bench-") as scratch:
        root = Path(scratch)
        (root / "bin").mkdir()
        # Use the same interpreter for all helpers, regardless of the caller's PATH.
        for name in ("pw-record", "notify-send", "wl-copy"):
            wrapper = root / "bin" / name
            wrapper.write_text(f"#!{sys.executable}\n"
                               f"import runpy\nrunpy.run_path({str(Path(__file__).resolve())!r})['helper']()\n")
            wrapper.chmod(0o755)
        for wav in args.wav:
            print(wav, flush=True)
            results = [[], []]
            for index in range(args.runs + 1):
                for i in (range(2) if index % 2 == 0 else (1, 0)):
                    result = trial(root, binaries[i], wav, args)
                    if index:
                        results[i].append(result)
            transcripts = {row[2] for rows in results for row in rows}
            if len(transcripts) != 1 or not next(iter(transcripts), "").strip():
                raise AssertionError(f"transcripts did not match: {transcripts!r}")
            for label, rows in zip(("baseline", "candidate"), results):
                start, stop = ([row[i] for row in rows] for i in range(2))
                print(f"  {label:9} start median {statistics.median(start):6.1f} ms "
                      f"[{min(start):.1f}–{max(start):.1f}]; "
                      f"stop → copy median {statistics.median(stop):6.1f} ms "
                      f"[{min(stop):.1f}–{max(stop):.1f}]", flush=True)
            print(f"  matched {next(iter(transcripts))!r}", flush=True)
            if args.check:
                # A deliberately stalled notification should not gate recorder launch.
                if max(row[0] for row in results[1]) >= args.notify_delay * 1000:
                    raise AssertionError("recorder startup waited for notification")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("baseline", type=Path)
    parser.add_argument("candidate", type=Path)
    parser.add_argument("model", type=Path)
    parser.add_argument("wav", type=Path, nargs="+")
    parser.add_argument("--runs", type=int, default=10)
    parser.add_argument("--notify-delay", type=float, default=0.15)
    parser.add_argument("--no-notify", action="store_true")
    parser.add_argument("--record-seconds", type=float, default=0.75)
    parser.add_argument("--language")
    parser.add_argument("--check", action="store_true", help="assert notification-independent startup")
    ARGS = parser.parse_args()
    if ARGS.runs < 1 or ARGS.notify_delay < 0 or ARGS.record_seconds < 0:
        parser.error("runs must be positive and delays nonnegative")
    if ARGS.check and (ARGS.no_notify or ARGS.notify_delay < 0.1):
        parser.error("--check requires enabled notifications with a delay of at least 0.1 seconds")
    main(ARGS)
