# Independent native output presenter control

October 28, 2026 · experiment branch `experiment/native-output-presenter`.

**The control continued reporting native output presentations while the eframe operator's UI callbacks paused.** During each of six native operator transitions, the operator callback gap was 552–575 ms and Metal reported 33–34 valid output presentations within that gap. The largest gap between valid presentation timestamps in the scored native-transition trial was **41.667 ms**, versus approximately **600 ms callback gaps** in the framework/full-app release baselines. Shorter pacing disturbances and zero timestamps remain; this is not a perfect-cadence claim.

This is an isolated presentation control, **not Rastera shared-canvas integration or an integrated-output fix**. Metal's timestamps are not physical scanout verification. First-canvas milestone acceptance remains pending.

## Checkpoint and preserved baseline

The previous investigation was committed and pushed separately as [`2a9238e9e26636e95e021301e2e3a86975e30d2c`](https://github.com/AlphaFatti/Rastera/commit/2a9238e9e26636e95e021301e2e3a86975e30d2c) on [`investigate/presentation-stall`](https://github.com/AlphaFatti/Rastera/tree/investigate/presentation-stall), then this experiment branch was created from it. The checkpoint does not signify acceptance.

Existing `rastera-core`, `rastera-desktop`, `framework-control`, shader, scheduling, viewport and timing code are unchanged from that checkpoint. Rust 1.99.1, eframe/egui 0.31.1, wgpu 24.0.5 and every existing lockfile package/dependency edge are unchanged. Cargo.lock adds only the new local workspace package `rastera-native-output`; it downloads no new Rust bridge library.

The new executable is `native-output-control`, in `crates/rastera-native-output`. Its eframe operator uses the baseline CPU timing helper and the same pinned Metal/FIFO configuration. Its one separate native window renders an original grid, moving marker and blinking stripe directly to its own drawable. Resizing changes that diagnostic viewport; there is no logical Rastera canvas in this executable.

## Native implementation and dependencies

A small Objective-C++ bridge uses the installed **AppKit, QuartzCore, Metal, Foundation, CoreFoundation, libdispatch and system C++ runtime**. `build.rs` invokes the installed `xcrun clang++` and `ar`, with C++17, ARC and blocks; native compilation uses `-Wall -Wextra -Werror`, and `-O3` for release (`-O0` for debug). There is no `cc`, Objective-C binding, Metal wrapper, downloaded SDK or other additional Rust dependency. A small MSL diagnostic shader is compiled through Metal's runtime compiler; full Xcode was not needed on this Mac.

This executable requires **macOS 14+ and an SDK containing CAMetalDisplayLink**. It does not silently substitute an eframe repaint/timer loop on older systems. Tested: Apple M1 Pro / macOS 15.5, Apple Command Line Tools with SDK 15.5 and Apple clang 17.0.0. Mach-O inspection reports native-control minimum OS 14.0; the unchanged baseline binaries remain linked with their existing minimum OS 11.0. Those link settings are not compatibility certification for other OS versions/hardware.

### Thread and ownership boundaries

- Main thread: creates/closes the `NSWindow`, creates/configures its layer-backed `NSView`, and performs required view operations. Resize/backing-scale callbacks update `contentsScale` and `drawableSize` on main, with implicit layer animations disabled. Rendering uses the actual supplied drawable's texture dimensions, not unsynchronized view state. Native assertions check these boundaries.
- Dedicated rendering thread: owns a `CAMetalDisplayLink` registered with that thread's current run loop. It requests 60 Hz and frame latency 2. Each callback uses the **update's drawable**, never eframe's callback or a main-thread `nextDrawable` call. Link registration, invalidation and delegate removal occur on this same thread. The render loop/callbacks have autorelease pools and never synchronously dispatch back to main.
- Metal resources: the control requests the system-default `MTLDevice` directly and creates a native command queue and CAMetalLayer/drawable pool. It shares no wgpu queue, device-fence bookkeeping, canvas texture, encoder or command buffer with eframe. Both paths still use the same physical GPU; they are not hardware-isolated.
- Bounded work: a nonblocking semaphore permits at most **two GPU command buffers in flight**, with a three-drawable layer pool. Backpressure skips an update instead of building a queue. Completion returns the slot. Drawables remain local to the update, handlers do not capture the drawable itself, and autorelease pools release temporary references promptly.
- Close/shutdown: a main-thread stop flag prevents new work, the rendering thread invalidates its link and joins, then a dispatch group drains outstanding GPU work (two-second drain timeout). Only afterward is the view detached. On a GPU-drain timeout, handlers still own their telemetry/semaphore/group state and do not touch AppKit. This avoids freeing callback storage early; it is not a tested GPU-hang recovery scheme. Normal thread join/drain behavior was exercised, not pathological driver blocking.
- Rust ownership: the opaque AppKit owner is `!Send`; only a separate C++ `shared_ptr` telemetry handle is transferred to the reporting thread. Stats contain atomics/mutex-protected POD events, not views/windows. Operator destruction joins the reporter before destroying the native owner. Closing just the output leaves the operator running; reopening requires restarting this diagnostic executable.

### Apple documentation consulted

The control follows the current [CAMetalDisplayLink API](https://developer.apple.com/documentation/quartzcore/cametaldisplaylink) and [delegate contract](https://developer.apple.com/documentation/quartzcore/cametaldisplaylinkdelegate/metaldisplaylink(_:needsupdate:)): finish encoding, **commit the command buffer, then call the drawable's ordinary `present()`**. It does not use timed presentation variants or `commandBuffer.presentDrawable` on this path. Registration uses the rendering thread's [run loop](https://developer.apple.com/documentation/quartzcore/cametaldisplaylink/add(to:formode:)); the run-loop object is used only on its own thread.

Apple's [custom Metal view sample](https://developer.apple.com/documentation/metal/creating-a-custom-metal-view) provides the custom NSView/CAMetalLayer and background-rendering precedent. Its macOS example uses CVDisplayLink; this control instead uses the current CAMetalDisplayLink API. The main-thread view boundary follows [Apple's NSView threading restrictions](https://developer.apple.com/library/archive/documentation/Cocoa/Conceptual/Multithreading/ThreadSafetySummary/ThreadSafetySummary.html). Drawables are held briefly, following [CAMetalLayer ownership guidance](https://developer.apple.com/documentation/quartzcore/cametallayer).

## Instrumentation: three different observations

The rendering/completion/presentation callbacks enqueue bounded POD records (capacity 4,096); the reporter drains them once per second. Callbacks do not write per-frame log text. Overflow is counted, and the analysis marks a lossy trace incomplete. Rust and native text share the C stdio FILE lock so clock anchors cannot interleave.

| Record | Observed time/meaning |
| --- | --- |
| `schedule` | CAMetalDisplayLink callback arrival; deadline and **estimated** target presentation time logged separately |
| `commit` | CPU return time and duration of `commandBuffer.commit`; preparation duration is separate |
| `complete` | Command completion handler arrival; Metal GPU start/end times and status are separate fields |
| `present_call` | CPU return time/duration of `drawable.present()`; this is **not** display presentation |
| `presented_handler` | Handler arrival in `callback_host_s`; drawable `presentedTime` in **`presented_host_s`** with an explicit validity flag |

The drawable handler is registered with [addPresentedHandler](https://developer.apple.com/documentation/metal/mtldrawable/addpresentedhandler(_:)). Apple's [presentedTime](https://developer.apple.com/documentation/metal/mtldrawable/presentedtime) documentation defines zero as not yet presented or a skipped frame. Zero, negative, nonfinite and unavailable samples are excluded from presentation-gap calculations and counted explicitly. Missing samples are **not** replaced with callback arrival, target timestamps, completion times, or zero-length gaps. If no valid timestamps exist, the summary reports presentation timing unavailable.

Offline analysis sorts positive presentation timestamps, because callback delivery need not establish timestamp order. It separately reports callback-arrival gaps, scheduling gaps, completion gaps and gaps between valid Metal-reported presentation times. A gap between valid samples may include unknown/skipped frames; it is not proof that a specific number of physical refreshes was missed. Apple-reported timing is the measurement boundary, not a camera/photodiode, monitor-output or scanout observation.

The native host clock and Rust `Instant` clock each have a one-time wall-clock anchor. These approximately align operator callback gaps, driver actions and native events; absolute alignment is not real-time calibrated. Presentation-gap subtraction itself uses only native presentation timestamps. CPU callbacks remain CPU measurements in all three executables; the two baseline executables have no actual presentation timestamps added in this experiment.

## Build and reproduction commands

From the repository root, using the pinned Rust toolchain and installed Command Line Tools:

```sh
cargo build --release --workspace --bins --locked
cargo run --release --locked -p rastera-native-output --bin native-output-control

# Existing egui command schedule: three operator minimize/restore cycles, then quit.
cargo run --release --locked -p rastera-native-output --bin native-output-control -- --minimize-test=operator
```

To reproduce the matched release suite, run these **serially**, without another renderer competing. Accessibility must be granted to the launching terminal/app (VS Code access was already granted here):

```sh
python3 tools/native_output_probe.py framework idle --out=/tmp/rastera-native-release/framework-idle.log
python3 tools/native_output_probe.py full idle --out=/tmp/rastera-native-release/full-idle.log
python3 tools/native_output_probe.py native idle --out=/tmp/rastera-native-release/native-idle.log
python3 tools/native_output_probe.py framework native --cycles=3 --out=/tmp/rastera-native-release/framework-native.log
python3 tools/native_output_probe.py full native --cycles=3 --out=/tmp/rastera-native-release/full-native.log
python3 tools/native_output_probe.py native native --cycles=3 --out=/tmp/rastera-native-release/native-native.log
python3 tools/native_output_probe.py framework egui --out=/tmp/rastera-native-release/framework-egui.log
python3 tools/native_output_probe.py full egui --out=/tmp/rastera-native-release/full-egui.log
python3 tools/native_output_probe.py native egui --out=/tmp/rastera-native-release/native-egui.log
python3 tools/native_output_probe.py native lifecycle --out=/tmp/rastera-native-release/native-lifecycle.log
python3 tools/native_output_probe.py native close-operator --out=/tmp/rastera-native-release/native-close-operator.log
python3 tools/summarize_native_output.py /tmp/rastera-native-release
```

The driver defaults to the **release** binaries and 12-second idle measurement. All builds used the same Rust release profile (`optimized`, no custom profile overrides) and the native bridge used `-O3`. Windows are arranged without overlap: operator `(20,40)`/`900×400`, outputs below it at `(20,500)` (and `(500,500)` for the second baseline output), each `450×280`. A two-second settle follows arrangement for idle/native modes.

Native mode presses the actual operator `AXMinimizeButton` through Accessibility, polls native minimized state, then restores with the native `AXMinimized=false` setter, checking every output remains unminimized. It repeats three cycles with three-second holds. All three apps confirmed all six requested operator states and exited normally. This is actual native-control invocation by automation, not a claim of human visual observation. For manual testing, use the yellow operator button and restore from the Dock while watching the output's marker/stripe.

The existing programmatic driver is unchanged (commands at approximately 3/6/9/12/15/18 seconds). Arrangement can overlap its first cycle, so the **scored egui comparison uses the common interval after app time 8 s**, covering cycles two and three. All three driver runs completed normally. Native measurement durations differ because Accessibility request/confirmation latency differs; these are matched-build reproduction tests, not throughput benchmarks.

Logs and adjacent `.events.jsonl` files are local in `/tmp/rastera-native-release`. A first native capture had an interleaved startup clock anchor and was excluded; after logging serialization, all native trials/lifecycle checks were rerun. Those initial native files remain under `initial-native/` for reference. Accepted derived measurements are preserved in [native-output-results.json](native-output-results.json); raw logs are temporary, not committed.

## Release comparison results

UI callback gaps are **not presentation gaps**. All numbers below are milliseconds in the scored measurement interval; baseline presentation fields are intentionally unmeasured.

| Executable / trial | Operator UI max gap | Baseline Left / Right UI max gaps | Native reported-presentation max gap |
| --- | ---: | ---: | ---: |
| Framework / idle | 17.246 | 17.033 / 17.412 | Not instrumented |
| Full app / idle | 28.299 | 28.108 / 27.938 | Not instrumented |
| Native control / idle | 17.888 | Separate native output | **25.000** |
| Framework / native operator transitions | 581.369 | 600.125 / 592.892 | Not instrumented |
| Full app / native operator transitions | 587.478 | 592.872 / 593.313 | Not instrumented |
| Native control / native operator transitions | 574.871 | Separate native output | **41.667** |
| Framework / egui driver, scored cycles 2–3 | 590.648 | 609.254 / 602.481 | Not instrumented |
| Full app / egui driver, scored cycles 2–3 | 585.598 | 592.333 / 592.449 | Not instrumented |
| Native control / egui driver, scored cycles 2–3 | 565.032 | Separate native output | **25.000** |

Separate native stages, again milliseconds:

| Native trial | Schedule callback max gap | Completion callback max gap | GPU end-time max gap | Presented-handler arrival max gap | Valid presentedTime max gap |
| --- | ---: | ---: | ---: | ---: | ---: |
| Idle | 23.016 | 22.127 | 22.164 | 25.036 | 25.000 |
| Native operator transitions | 33.339 | 33.379 | 33.373 | 41.647 | 41.667 |
| Eg​​ui operator transitions, scored cycles 2–3 | 22.583 | 22.489 | 22.486 | 24.808 | 25.000 |

The accepted native-transition interval spans 35.584 s and contains **2,127 valid presentation samples**, with **five zero-timestamp callbacks** in the interval. Idle: 720 valid samples over 12.005 s, no zero samples in the interval. Scored programmatic trial: 793 valid samples over 13.249 s, no zero samples in that interval. Across the entire corresponding processes, startup/resize/shutdown also produced zero samples (7 idle, 13 native-transition, 6 egui); unavailable/nonfinite timestamp counters were zero. Zero samples were not treated as positive presentation evidence.

### Presentations during each native operator callback pause

This aligns each longest operator callback gap with valid native presentation timestamps. The final column includes 100 ms of surrounding context on each side; these are reported presentation gaps, not guesses from callback arrival.

| Operator gap starts (app seconds) | UI gap | Valid output presentations during UI gap | Max surrounding presentation gap |
| ---: | ---: | ---: | ---: |
| 5.529794 | 567.103 ms | 34 | 16.667 ms |
| 11.601221 | 557.578 ms | 33 | 16.667 ms |
| 17.422576 | 565.492 ms | 34 | 16.667 ms |
| 23.455034 | 552.945 ms | 33 | 25.000 ms |
| 29.255461 | 574.871 ms | 34 | 16.667 ms |
| 35.305914 | 552.062 ms | 33 | 16.667 ms |

The two scored programmatic cycles likewise produced 33–34 valid presentations inside each approximately 551–565 ms operator gap, with 16.667 ms maximum surrounding gaps.

### Remaining disturbances

This is not perfect 60 Hz delivery. The scored native-transition trial's largest gap between valid presentation reports was **41.667 ms**, from app time approximately `16.234283` to `16.275949` s (display-link sequences 969 to 971). Sequence 970 returned a zero presentation timestamp, so its presentation is unknown/skipped; no callback time was substituted. This gap occurred outside the six measured long operator callback pauses. The native schedule/completion maxima were approximately 33.3 ms. Idle and scored programmatic trials also showed 25 ms presentation gaps.

Setup/window arrangement was excluded from scoring and included a longer 66.667 ms valid-timestamp gap in the native-transition process, with three intervening zero samples. Native `commit` calls reached 6.552 ms (idle), 5.925 ms (native transitions), and 5.216 ms (egui); `present()` calls reached 2.833, 4.421 and 3.055 ms respectively. These are CPU call durations, not output presentation times.

The full-app release driver did **not** reproduce the historical debug run's hundreds-of-milliseconds queue-submit stall: scored submit maximum was 4.945 ms; the whole process's maximum was 7.712 ms. UI callback pauses still reproduced. This does not establish that release optimization fixes the shared path; profile, timing, and scheduling differ. The prior debug finding remains a separate observation.

## Lifecycle and validation

The accepted native lifecycle run resized the output to `700×360`, `350×500` and `800×220` native window sizes. Backing-pixel updates were logged from main-thread view callbacks, rendering resumed, and no GPU error was recorded. It then minimized/restored the **output itself** twice (separate from operator tests), closed the output while leaving the operator responsive for three seconds, and closed the operator. A separate operator-first close left the output open until application shutdown. Both exited with code 0.

Output-hidden/resize testing intentionally has gaps and zero timestamps and is not scored as continuous-visible-output performance: its largest valid presentation gap was approximately 4.03 s while the output was hidden, and 37 zero samples occurred in its measurement interval. It recovered on restore. In all accepted native runs, `max_inflight=1` (below the enforced bound of 2), backpressure=0, lost telemetry=0, GPU errors=0. Every stop reported GPU drain complete and zero GPU work in flight. These tests do not force saturation, simulate a GPU hang, or certify long-term memory/leak behavior.

Passed:

```sh
cargo build --release --workspace --bins --locked --offline
cargo fmt --all -- --check
cargo check --workspace --all-targets --locked --offline
cargo test --workspace --locked --offline
cargo clippy --workspace --all-targets --locked --offline -- -D warnings
python3 -m unittest discover -s tools -p 'test_*.py'
```

Four Rust tests and three focused Python analysis tests pass. Analysis tests cover out-of-order handler arrival, zero/nonfinite/unavailable timestamps, refusal to invent a zero presentation gap when timestamps are unavailable, and trace-loss marking. Objective-C++ compiles with warnings as errors. The existing transitive `block` 0.1.6 future Rust compatibility warning remains; no dependency upgrade was made.

## Interpretation and limits

The separate native presenter provides positive **Metal-reported presentation** evidence through operator transitions despite paused eframe UI callbacks. It is a useful candidate for the next integration experiment, not proof that Rastera's existing preview/output pipeline is fixed. This control does not consume the shared canvas, synchronize cross-queue texture access, implement output routing, or verify production behavior.

Only one Mac, one short sequential run per accepted case, one native output versus two baseline viewports, and a light diagnostic shader were tested. The GPU and compositor are still shared physical resources. Display refresh/thermal/load conditions were not controlled, trial order was not randomized, and no camera/photodiode, scanout trace, genlock, external-projector matrix, hot-plug, prolonged show, or calibrated latency test was performed. Minimize-all/restore-all acceptance and the prior stricter automation case remain unverified. The zero timestamps, remaining pacing gaps, and lack of physical scanout verification must remain visible in any conclusion. Overall first-canvas acceptance is pending.
