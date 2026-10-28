# Presentation stall: controlled comparison

October 28, 2026 · base `3aa156d7811eafe8c5ffaa591f976ba83ba4fbcc` · branch `investigate/presentation-stall`.

**The pause reproduces without Rastera's shared canvas.** The framework-only control exhibits approximately 0.6-second gaps in visible projection UI callbacks when the operator is minimized/restored. Both the egui driver and actual native minimize-button invocation reproduce it. The full application's native-button trial has similar UI gaps while its producer maintains near-target cadence. The egui-driven restore additionally stalls the full application's queue submission. These are two observed effects, not proof of two independent root causes. Milestone acceptance remains pending; no presentation backend replacement or stall fix is included.

## Fixed conditions and controls

Host: Apple M1 Pro, arm64, macOS 15.5. All three executables report Metal on that adapter. Rust 1.99.1, eframe/egui/egui-wgpu 0.31.1, wgpu 24.0.5 and the entire Cargo.lock remain unchanged from the WIP. All measurements use the default **debug, unoptimized + debuginfo** build; none uses release. No competing Rastera process was present at the start, and runs were sequential. Compilation/checks were performed outside the measurement runs.

- **Headless producer:** `rastera-core` binary uses its existing 3840 × 1080 shader, texture, producer thread, 60 Hz deadline policy, and submission-specific completion wait. It requests a Metal adapter/device without a compatible surface. There are no windows, surfaces, egui objects, or UI notifications. The API's frame-ready callback is a no-op. Initialization uses a small standard-library future executor, with no new dependency.
- **Framework control:** desktop binary uses only eframe/egui/wgpu and shared CPU timing instrumentation. It creates the operator and two deferred projection viewports with the same initial sizes, Metal/FIFO configuration and framework defaults as the full application. Yellow circles animate from monotonic elapsed time through egui repaint timers. There is no `rastera-core` use, canvas texture, shader generator, or custom frame producer. Its one diagnostic thread only reports counters and optionally sends window commands. `nm` found **zero** `rastera_core` symbols in this debug binary, versus 186 in the full application. The desktop package still declares the core dependency for its full-app binary; the control does not reference or link it.
- **Full application:** WIP renderer, canvas geometry, shaders, viewports, frame-ready scheduling and GPU wait policy are preserved. Only timing/instrumentation is changed. The package adds `default-run = "rastera-desktop"` to preserve the existing run command now that a second binary exists.

This comparison removes the entire canvas producer/shared-texture path from the framework control. It does not separately remove egui, winit, AppKit, Metal surfaces, the compositor, or the OS animation. The controls' workloads and repaint sources intentionally differ, so they establish reproduction/association rather than a precise cost comparison.

## Corrected measurements

The WIP timed `queue.submit([encoder.finish()])` as a single stage. Its historical “submit” maxima could include encoder finalization and cannot distinguish those calls. Historical lifetime callback maxima also lost when a gap happened. Those figures remain a historical record, not stage attribution.

The producer now records independent CPU durations for:

| Stage | Measurement |
| --- | --- |
| `cadence` | Consecutive producer frame-start instants, including pacing and missed deadlines |
| `record` | Uniform write, encoder creation, render-pass recording and pass closure |
| `finish` | Only `encoder.finish()` |
| `submit` | Only `queue.submit([already_finished_command])` |
| `wait` | Only `device.poll(WaitForSubmissionIndex(producer_submission))` |
| `notify` | Only the frame-ready callback; no-op in headless |

Each roughly one-second report drains a bounded, fixed-size interval accumulator and prints sample count, total CPU duration, maximum duration and that maximum's **monotonic start timestamp**. A call spanning report boundaries appears once it completes, possibly with a start in a previous interval. Counts/timestamps should be used together, not just maxima. Stage durations truncate to integer microseconds; zero does not mean literally zero execution time.

The independent monitor also reports cumulative submissions/completions, current producer phase and phase-start timestamp. Phases are `0` pacing, `1` recording, `2` submitting, `3` completion wait, `4` notification, `5` encoder finish. An unfinished stall remains identifiable even before a completed duration is reported. Per-viewport interval reports include UI-entry/return counts, maximum callback gap and its start time, plus CPU paint-pass callback entries. Aggregates reset each report; storage does not grow with frame count.

**None of these is a display-presentation timestamp.** Paint-pass entry only means CPU encoding reached the callback. The observed producer counter is not a guaranteed displayed frame ID, UI pass counts are not display FPS, and the GPU completion wait's CPU duration is not GPU execution duration. egui may run more than one UI pass per producer frame. macOS minimized flags in egui remain cached and are not authoritative native window state.

App timestamps share an `Instant` origin within each process. Each process logs a one-time `(mono_us, unix_us)` clock anchor; external driver events use a separate monotonic origin with a wall-clock anchor for approximate alignment. Native action request/confirmation events bracket Accessibility latency as well as the actual transition; they are not exact AppKit animation-start/end timestamps.

## Commands and procedure

From the repository root, with the pinned toolchain available:

```sh
cargo build --workspace --bins --locked

# Direct controls for interactive observation:
cargo run --locked -p rastera-core --bin headless-producer -- --seconds=24
cargo run --locked -p rastera-desktop --bin framework-control
cargo run --locked -p rastera-desktop -- --outputs --diagnostics
```

The measured suite used these commands, serially, against the already-built debug executables:

```sh
python3 tools/presentation_probe.py headless idle --seconds=24 --out=/tmp/rastera-presentation/headless-idle.log
python3 tools/presentation_probe.py framework idle --seconds=12 --out=/tmp/rastera-presentation/framework-idle.log
python3 tools/presentation_probe.py framework egui --out=/tmp/rastera-presentation/framework-egui.log
python3 tools/presentation_probe.py framework native --cycles=3 --out=/tmp/rastera-presentation/framework-native.log
python3 tools/presentation_probe.py full idle --seconds=12 --out=/tmp/rastera-presentation/full-idle.log
python3 tools/presentation_probe.py full egui --out=/tmp/rastera-presentation/full-egui.log
python3 tools/presentation_probe.py full native --cycles=3 --out=/tmp/rastera-presentation/full-native.log
python3 tools/summarize_presentation.py /tmp/rastera-presentation
```

The egui mode passes `--minimize-test=operator`: minimize at approximately 3/9/15 seconds, restore at 6/12/18 seconds, close normally after 21 seconds. The framework control implements the same three-cycle schedule. Both outputs remain open.

The native mode needs Accessibility access for the launching terminal/app (VS Code access was granted). It places the operator at `(20,40)` sized `900×400`, Left at `(20,500)` sized `450×280`, and Right at `(500,500)` sized `450×280`, without overlap. It waits three seconds after arranging windows. For each of three cycles it **presses the native `AXMinimizeButton`**, confirms `AXMinimized=true`, holds, then restores with the native Accessibility `AXMinimized=false` setter and confirms it. Each confirmation also checks both projection windows are unminimized. The two native runs confirmed all six requested operator states each, without timeout. This exercises native controls through Accessibility; it is not a claim of human mouse-click or visual acceptance testing. Manual reproduction remains: click the yellow operator button, observe both projection circles/markers during the animation and while minimized, then restore from the Dock.

Raw app logs and adjacent `.events.jsonl` action records are in `/tmp/rastera-presentation` on the test Mac; they are temporary and not committed. The report records the principal numeric results and representative event times. The checked-in scripts reproduce the collection and interval summary. Headless and egui-driver runs exited normally with code 0. The runner intentionally terminates only its owned processes after idle/native runs; those cases do not test graceful shutdown.

## Results

Maximum UI callback gaps below are in **milliseconds**, after a two-second startup exclusion. These are maxima aggregated from fresh interval samples, not display-frame intervals. Native runs lasted approximately 56.3 s (framework) and 43.0 s (full), including Accessibility overhead; egui runs lasted approximately 21 s. Idle runs were 12 s. The headless run sampled approximately 24.1 s; its last sample recorded 1,445 completed producer frames.

| Run | Operator max gap | Visible Left max gap | Visible Right max gap |
| --- | ---: | ---: | ---: |
| Framework idle | 16.492 | 16.708 | 16.610 |
| Framework egui driver | 616.114 | 635.533 | 629.300 |
| Framework native controls | 587.106 | 599.466 | 596.119 |
| Full idle | 22.956 | 22.947 | 27.310 |
| Full egui driver | 627.953 | 639.830 | 632.475 |
| Full native controls | 590.466 | 603.211 | 604.363 |

Fresh interval data shows the effect on **each** native transition, rather than just a lifetime maximum. Left's gap durations, in minimize/restore order across three cycles:

| Native trial | Cycle 1 min / restore | Cycle 2 min / restore | Cycle 3 min / restore |
| --- | --- | --- | --- |
| Framework | 574.378 / 547.493 ms | 589.506 / 551.816 ms | 599.466 / 570.810 ms |
| Full | 569.608 / 573.112 ms | 603.211 / 569.935 ms | 589.062 / 571.337 ms |

Producer cadence and separate stage maxima, in **milliseconds**:

| Run | Mean cadence | Max cadence | Record max | Finish max | Submit max | Wait max | Notify max |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| Headless idle | 16.704 | 43.068 | 24.511 | 0.070 | 6.050 | 12.294 | 0.073 (no-op) |
| Full idle | 16.673 | 23.401 | 1.727 | 0.019 | 5.441 | 4.272 | 1.226 |
| Full egui driver | 17.836 | 495.871 | 6.087 | 0.085 | 468.771 | 6.374 | 3.906 |
| Full native controls | 16.693 | 43.701 | 5.045 | 0.191 | 16.630 | 7.742 | 0.634 |

The headless control had no window transitions, and no approximately 0.6-second cadence interruption appeared during its short run. The full native trial maintained near-60 Hz **producer** cadence while projection UI callbacks paused. That does not mean the projections displayed those frames during the pause.

The full egui-driver restore path adds three distinct queue-submission stalls. Encoder finish stayed short; it is now measured separately. Times below are relative to that process's monotonic origin:

| `submit` start | Submit duration | Report timestamp | Related start-to-start cadence gap |
| ---: | ---: | ---: | ---: |
| 6.656493 s | 432.313 ms | 8.061040 s | 457.139 ms |
| 12.713980 s | 440.743 ms | 14.106550 s | 462.691 ms |
| 18.729172 s | 468.771 ms | 20.162587 s | 495.871 ms |

For example, the largest submission starts at `18.729172 s`, after the third programmatic restore request; it is recorded in the interval ending at `20.162587 s`. In that interval the submit maximum is `468771 us`, while the other stages remain separately bounded as in the table. This is actual queue-call delay in this driven case, not the previous conflation with encoder finish. The native-button run did **not** reproduce a comparable producer submit stall.

## Interpretation and limits

1. The visible-output callback pause is reproducible in the framework-only control, with both driving methods. The custom producer and shared offscreen texture are **not necessary** for that pause. This points the next experiment toward the framework/native presentation path, rather than first replacing Rastera's generator.
2. The full application also exposes an additional queue-submission/cadence interruption during egui-driven restore. It does not appear as a comparable interruption in the headless or full native-button trials. Shared-device/presentation contention is a candidate explanation, consistent with the pinned source investigation, but these measurements alone do not identify the held lock or prove causation. Do not generalize the egui-driven submit stall to every native minimize action.
3. This experiment does not distinguish egui from eframe/winit/AppKit/Metal/compositor behavior. There is no new stack capture aligned to each timed event, GPU timestamp query, compositor trace, high-speed recording, or physical scanout measurement. Independent callbacks still share eframe's event thread. Actual display-presentation timing and visual continuity remain unverified.
4. There is one Mac, one debug build, one sequential trial per case with three transitions of each kind, fixed run order, and no randomized/thermal/refresh-rate control or endurance test. Native and egui-driver trials use different scheduling/command paths; idle/egui cases use framework-default window placement, while native cases share the explicit layout. Timing instrumentation itself adds CPU/locking overhead; there is no matched uninstrumented performance trial. These are reproduction controls, not comparative throughput benchmarks. The headless control was not run concurrently with OS window transitions.
5. This task tests only operator transitions with projections open. It does not resolve the historical stricter all-window automation timeout or certify minimize-all/restore-all behavior. Human visual retesting and milestone acceptance remain pending.

The next measured experiment should isolate presentation/acquisition from the shared event thread while keeping these controls and clocks available. A backend replacement is deliberately left for that experiment.

## Validation

Passed: `cargo build --workspace --bins --locked --offline`, `cargo fmt --all -- --check`, `cargo check --workspace --all-targets --locked --offline`, `cargo test --workspace --locked --offline` (four tests), and `cargo clippy --workspace --all-targets --locked --offline -- -D warnings`. A new focused timing test verifies interval reset and preservation of the slow call's start timestamp; existing geometry and bounded-repaint tests remain. The pre-existing transitive `block` 0.1.6 future Rust compatibility warning persists. Python driver/summary sources were syntax-checked and exercised against the collected logs. `Cargo.lock`, the pinned toolchain and all dependency version declarations match the WIP exactly.
