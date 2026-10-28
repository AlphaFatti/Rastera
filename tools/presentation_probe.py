#!/usr/bin/env python3
"""Run one owned diagnostic process. Native mode needs macOS Accessibility.

Logs are stdout/stderr from the app plus driver events timestamped with
Python's monotonic clock relative to process launch. App clocks start later;
use the adjacent native action request/confirmation bracket when correlating.
"""
import argparse
import json
from pathlib import Path
import subprocess
import time


def apple_event(pid, statements):
    script = f'''tell application "System Events"
set targetProcess to first application process whose unix id is {pid}
tell targetProcess
{statements}
end tell
end tell'''
    result = subprocess.run(["osascript", "-e", script], capture_output=True,
                            text=True, timeout=15, check=True)
    return result.stdout.strip()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("app", choices=["headless", "framework", "full"])
    parser.add_argument("mode", choices=["idle", "egui", "native"])
    parser.add_argument("--seconds", type=int, default=24)
    parser.add_argument("--cycles", type=int, default=3)
    parser.add_argument("--out", required=True, type=Path)
    args = parser.parse_args()
    if args.app == "headless" and args.mode != "idle":
        parser.error("headless only supports idle")
    binary = {"headless": "headless-producer", "framework": "framework-control",
              "full": "rastera-desktop"}[args.app]
    command = [str(Path("target/debug", binary).resolve())]
    if args.app == "headless":
        command += [f"--seconds={args.seconds}"]
    elif args.app == "full":
        command += ["--outputs", "--diagnostics"]
    if args.mode == "egui":
        command += ["--minimize-test=operator"]
    args.out.parent.mkdir(parents=True, exist_ok=True)
    events = args.out.with_suffix(".events.jsonl")
    origin = time.monotonic_ns()
    origin_unix = time.time_ns()
    with args.out.open("w") as log, events.open("w") as event_log:
        def event(kind, **data):
            item = dict(t_us=(time.monotonic_ns()-origin)//1000,
                        unix_us=(origin_unix+time.monotonic_ns()-origin)//1000,
                        event=kind, **data)
            event_log.write(json.dumps(item)+"\n")
            event_log.flush()
            print(json.dumps(item), flush=True)

        proc = subprocess.Popen(command, stdout=log, stderr=subprocess.STDOUT)
        event("launch", pid=proc.pid, command=command)
        try:
            if args.mode == "native":
                time.sleep(3)
                # Same non-overlapping positions/sizes in both windowed apps.
                apple_event(proc.pid, '''set frontmost to true
set position of window "Rastera" to {20, 40}
set size of window "Rastera" to {900, 400}
set position of window "Rastera — Left" to {20, 500}
set size of window "Rastera — Left" to {450, 280}
set position of window "Rastera — Right" to {500, 500}
set size of window "Rastera — Right" to {450, 280}''')
                event("layout_ready")
                time.sleep(3)
                for cycle in range(1, args.cycles+1):
                    for minimized in [True, False]:
                        event("native_request", cycle=cycle, minimized=minimized)
                        action = '''perform action "AXPress" of (first button of targetWindow whose subrole is "AXMinimizeButton")''' if minimized else '''set value of attribute "AXMinimized" of targetWindow to false'''
                        expected = str(minimized).lower()
                        result = apple_event(proc.pid, f'''set targetWindow to first window whose name is "Rastera"
{action}
repeat 40 times
if (value of attribute "AXMinimized" of targetWindow) is {expected} then exit repeat
delay 0.1
end repeat
return {{(value of attribute "AXMinimized" of targetWindow), (value of attribute "AXMinimized" of window "Rastera — Left"), (value of attribute "AXMinimized" of window "Rastera — Right")}}''')
                        event("native_confirm", cycle=cycle, minimized=minimized, result=result)
                        if result != expected + ", false, false":
                            raise RuntimeError("Native state not confirmed or a projection was minimized")
                        time.sleep(3)
            elif args.mode == "egui":
                proc.wait(timeout=30)
                if proc.returncode:
                    raise RuntimeError(f"App exited with {proc.returncode}")
            elif args.app == "headless":
                proc.wait(timeout=args.seconds+15)
                if proc.returncode:
                    raise RuntimeError(f"Headless exited with {proc.returncode}")
            else:
                time.sleep(args.seconds)
            event("done", returncode=proc.poll())
        finally:
            if proc.poll() is None:
                # Stop only the process created by this run. Native-mode termination
                # is not a graceful-shutdown test; egui mode closes normally.
                proc.terminate()
                try:
                    proc.wait(timeout=3)
                except subprocess.TimeoutExpired:
                    proc.kill()
                    proc.wait()


if __name__ == "__main__":
    main()
