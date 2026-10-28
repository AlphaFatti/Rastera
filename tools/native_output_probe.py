#!/usr/bin/env python3
"""Matched release controls plus native output resize/close/shutdown checks.
Requires macOS Accessibility. Baseline executables are unchanged.
"""
import argparse
import json
from pathlib import Path
import subprocess
import time
import threading
from presentation_probe import apple_event


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("app", choices=["framework", "full", "native"])
    parser.add_argument("mode", choices=["idle", "native", "egui", "lifecycle", "close-operator"])
    parser.add_argument("--target", choices=["operator", "picture"], default="operator")
    parser.add_argument("--capture", action="store_true", help="Capture only the other app window; requires Screen Recording and /tmp/rastera-window-info")
    parser.add_argument("--window-animation", choices=["default", "none"], default="default")
    parser.add_argument("--profile", choices=["debug", "release"], default="release")
    parser.add_argument("--cycles", type=int, default=3)
    parser.add_argument("--seconds", type=int, default=12)
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    if args.mode in ["lifecycle", "close-operator"] and args.app != "native":
        parser.error("lifecycle modes apply only to native")
    binary = {"framework": "framework-control", "full": "rastera-desktop", "native": "native-output-control"}[args.app]
    outputs = ["Rastera — Native Output"] if args.app == "native" else ["Rastera — Left", "Rastera — Right"]
    command = [str(Path("target", args.profile, binary).resolve())]
    if args.app == "native":
        command += ["--window-animation="+args.window_animation]
    if args.app == "full":
        command += ["--outputs", "--diagnostics"]
    if args.mode == "egui":
        command += ["--minimize-test=operator"]
    if args.capture and (args.app != "native" or args.mode not in ["native", "idle"]):
        parser.error("capture requires native native or native idle")
    if args.target == "picture" and (args.app != "native" or args.mode not in ["native", "idle"]):
        parser.error("picture target requires native native or native idle")
    origin = time.monotonic_ns()
    origin_unix = time.time_ns()
    args.out.parent.mkdir(parents=True, exist_ok=True)
    with args.out.open("w") as log, args.out.with_suffix(".events.jsonl").open("w") as event_log:
        def event(kind, **data):
            now = time.monotonic_ns()-origin
            item = dict(t_us=now//1000, unix_us=(origin_unix+now)//1000, event=kind, **data)
            event_log.write(json.dumps(item)+"\n")
            event_log.flush()
            print(json.dumps(item), flush=True)

        def set_minimized(pid, title, minimized):
            action = 'perform action "AXPress" of (first button of targetWindow whose subrole is "AXMinimizeButton")' if minimized else 'set value of attribute "AXMinimized" of targetWindow to false'
            expected = str(minimized).lower()
            checks = ", ".join(f'(value of attribute "AXMinimized" of window "{output}")' for output in outputs)
            result = apple_event(pid, f'''set targetWindow to first window whose name is "{title}"
{action}
repeat 40 times
if (value of attribute "AXMinimized" of targetWindow) is {expected} then exit repeat
delay 0.1
end repeat
return {{(value of attribute "AXMinimized" of targetWindow), {checks}}}''')
            if title == "Rastera" and result != ", ".join([expected]+["false"]*len(outputs)):
                raise RuntimeError(f"Unexpected native states: {result}")
            if result.split(", ")[0] != expected:
                raise RuntimeError(f"Target state mismatch: {result}")
            if title != "Rastera":
                operator_state = apple_event(pid, 'return value of attribute "AXMinimized" of window "Rastera"')
                if operator_state != "false":
                    raise RuntimeError(f"Operator unexpectedly minimized: {operator_state}")
            return result

        def close(pid, title):
            apple_event(pid, f'perform action "AXPress" of (first button of window "{title}" whose subrole is "AXCloseButton")')

        proc = subprocess.Popen(command, stdout=log, stderr=subprocess.STDOUT)
        event("launch", pid=proc.pid, command=command, profile=args.profile)
        try:
            time.sleep(1)
            layout = ['set frontmost to true', 'set position of window "Rastera" to {20, 40}', 'set size of window "Rastera" to {900, 400}']
            for i, output in enumerate(outputs):
                layout += [f'set position of window "{output}" to {{{20+i*480}, 500}}', f'set size of window "{output}" to {{450, 280}}']
            apple_event(proc.pid, "\n".join(layout))
            event("layout_ready")
            if args.mode != "egui":
                time.sleep(2)
            capture_stop = threading.Event()
            capture_thread = None
            if args.capture:
                info = json.loads(subprocess.check_output(["/tmp/rastera-window-info", str(proc.pid)], text=True))
                if not info["screen_capture_allowed"]:
                    raise RuntimeError("Screen Recording not granted")
                observed = "Rastera — Native Output" if args.target == "operator" else "Rastera"
                window_id = next(w["id"] for w in info["windows"] if w["title"] == observed)
                capture_dir = args.out.with_suffix(".frames")
                capture_dir.mkdir(exist_ok=True)
                def capture():
                    timing = capture_dir / "timing.jsonl"
                    while not capture_stop.is_set():
                        started = time.monotonic_ns()-origin
                        dest = capture_dir/f"{started//1000:012d}.png"
                        result = subprocess.run(["/usr/sbin/screencapture", "-x", "-o", "-l", str(window_id), str(dest)], capture_output=True)
                        ended = time.monotonic_ns()-origin
                        with timing.open("a") as capture_timing:
                            capture_timing.write(json.dumps(dict(file=dest.name, start_us=started//1000,
                                                                 end_us=ended//1000, returncode=result.returncode))+"\n")
                        if result.returncode:
                            (capture_dir/"error.txt").write_bytes(result.stderr)
                            break
                        capture_stop.wait(0.10)
                capture_thread = threading.Thread(target=capture)
                capture_thread.start()
                event("capture_start", observed_title=observed, window_id=window_id)
            event("measure_start")
            if args.mode == "idle":
                time.sleep(args.seconds)
            elif args.mode == "native":
                for cycle in range(1, args.cycles+1):
                    for minimized in [True, False]:
                        event("native_request", cycle=cycle, minimized=minimized, target=args.target)
                        state = set_minimized(proc.pid, "Rastera" if args.target == "operator" else outputs[0], minimized)
                        event("native_confirm", cycle=cycle, minimized=minimized, result=state)
                        time.sleep(3)
            elif args.mode == "egui":
                # Driver starts at launch (3/6/9/12/15/18s); use cycles 2/3 for
                # matched analysis if initial window arrangement overlaps cycle 1.
                proc.wait(timeout=30)
                if proc.returncode:
                    raise RuntimeError(f"Driver exit={proc.returncode}")
            elif args.mode == "lifecycle":
                for width, height in [(700, 360), (350, 500), (800, 220)]:
                    event("output_resize", width=width, height=height)
                    apple_event(proc.pid, f'set size of window "{outputs[0]}" to {{{width}, {height}}}')
                    time.sleep(2)
                for minimized in [True, False, True, False]:
                    event("output_minimize_request", minimized=minimized)
                    state = set_minimized(proc.pid, outputs[0], minimized)
                    event("output_minimize_confirm", result=state)
                    if state != ", ".join([str(minimized).lower()]*2):
                        raise RuntimeError(f"Output state mismatch: {state}")
                    time.sleep(2)
                event("close_output_request")
                close(proc.pid, outputs[0])
                time.sleep(3)
                event("close_operator_request")
                close(proc.pid, "Rastera")
                proc.wait(timeout=8)
                if proc.returncode:
                    raise RuntimeError(f"Lifecycle exit={proc.returncode}")
            elif args.mode == "close-operator":
                event("close_operator_request")
                close(proc.pid, "Rastera")
                proc.wait(timeout=8)
                if proc.returncode:
                    raise RuntimeError(f"Operator-first exit={proc.returncode}")
            if capture_thread:
                capture_stop.set()
                capture_thread.join(timeout=5)
            event("measure_end")
            if proc.poll() is None:
                # Graceful exit for all windowed controls; also captures final native
                # stop/drain telemetry instead of killing the output renderer.
                event("close_operator_request")
                close(proc.pid, "Rastera")
                proc.wait(timeout=8)
                if proc.returncode:
                    raise RuntimeError(f"Close exit={proc.returncode}")
            event("done", returncode=proc.returncode)
        finally:
            if "capture_stop" in locals():
                capture_stop.set()
            if "capture_thread" in locals() and capture_thread:
                capture_thread.join(timeout=5)
            if proc.poll() is None:
                proc.terminate()
                try:
                    proc.wait(timeout=3)
                except subprocess.TimeoutExpired:
                    proc.kill()
                    proc.wait()


if __name__ == "__main__":
    main()
