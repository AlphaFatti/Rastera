# Independent native output presenter control

October 28, 2026 · experiment branch `experiment/native-output-presenter`.

**The control continued reporting native output presentations while the eframe operator's UI callbacks paused.** During each of six native operator transitions, the operator callback gap was 552–575 ms and Metal reported 33–34 valid output presentations within that gap. The largest gap between valid presentation timestamps in the scored native-transition trial was **41.667 ms**, versus approximately **600 ms callback gaps** in the framework/full-app release baselines. Shorter pacing disturbances and zero timestamps remain; this is not a perfect-cadence claim.

**Confirmed remaining visual failure:** minimizing `Rastera — Native Output` briefly freezes the still-visible yellow-circle Metal controls (`Rastera`), including their displayed timestamps. The A/B/C animation-settings follow-up below reproduces this failure under all three conditions.

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

## Content / window-direction follow-up (October 28, 2026)

**Acceptance remains pending.** The pre-change native experiment was committed and pushed on `experiment/native-output-presenter` as `d1f3b56f4387fa7e349d10f23451dd48a23b8474` (`WIP: independent native presenter with pending visual acceptance`). The changes below form the subsequent WIP content-instrumentation checkpoint; the confirmed visual transition failure remains unresolved. Baseline renderer, shared canvas, geometry, framework and dependency versions remain unchanged.

### Exact identities and user observation

| Native title | Role and image | Source of animation |
| --- | --- | --- |
| `Rastera` | eframe/wgpu **Metal controls**, moving yellow circle | Rust `Instant` sampled by main-thread `Control::update` |
| `Rastera — Native Output` | independent CAMetalLayer **picture**, grid/marker/stripe | native rendering-thread monotonic clock |

The user clarified that the freeze ends **when the macOS minimize animation ends**, rather than persisting until restoration. They subsequently identified the “metal” window as the freezing window and requested visible timestamps there. Both directions were tested separately, including minimizing the picture while observing the yellow-circle controls. Terminology in earlier discussion was ambiguous; the titles above are the identities used in the evidence.

The controls now display a large monotonic elapsed timestamp, UI callback counter, and Unix milliseconds sampled in the UI callback. These values intentionally stop updating when that callback stops: they describe the visible controls' content, not background liveness or display presentation. The circle has the same UI-derived clock. No claim is made that controls animation is independent of AppKit transitions.

### Native content trace

No operator-to-native animation-state coupling was found. Before this change, `needsUpdate` already computed animation from `CACurrentMediaTime()` on its dedicated run-loop thread and uploaded fragment bytes there; the fourth float (sequence) was unused in the shader. The picture does not sample an operator-generated texture. Its only image texture is each supplied drawable's render target.

The native worker now establishes an explicit `std::chrono::steady_clock` origin. Each successful encoding increments a rendering-thread-owned 64-bit **content ID**. A typed 32-byte uniform carries elapsed time, actual drawable dimensions, and integer content identity. The shader renders the low 32 bits as ten large decimal digits and a 32-cell binary stripe (least significant bit left). At 60 Hz the visible ID wraps after roughly 2.27 years; logs retain all 64 bits. This is diagnostic picture content, not an editor overlay or Rastera canvas integration.

`native_content_link` records associate content encoding, commit, completion, present-call and presented-handler records with the same captured-by-value content ID and drawable ID. Completion proves execution finished, not that a frame was visible. A valid drawable timestamp is still Apple API evidence, not physical scanout. The rendering thread owns the clock/counter/uniform update/encoding; driver threads deliver completion/presentation callbacks; the reporter only drains POD telemetry. AppKit window/view operations remain on main. No new bridge dependencies, queue synchronization, layer ownership, or presentation ordering changes were introduced.

The summary now joins the entire trace by sequence and reports missing completions, missing present calls, missing handlers, frames without valid presentation evidence, and mismatched content/drawable links. Missing or zero timestamps remain unknown/skipped outcomes. A shutdown drain waits for GPU completion, not every presentation handler; terminal missing presentation callbacks are retained rather than silently marked presented.

### Separate direction reproductions

Matched release builds, same Mac and pinned versions as above. Screen Recording was granted and verified after restarting VS Code. Accessibility invokes the native yellow minimize button (`AXPress`), polls the target state and restores through native `AXMinimized=false`; these are native controls exercised by automation, **not human clicks or human acceptance**. The operator and picture remain in their nonoverlapping fixed layout. Two cycles per direction, three-second confirmed holds, separate processes. Window captures target only the *other* Rastera window by its Window Server ID.

| Trial | Max controls UI gap | Max valid picture presentedTime gap | Whole-process zero callbacks | Missing presented handlers / committed frames |
| --- | ---: | ---: | ---: | ---: |
| Minimize controls, capture picture | 602.192 ms | 41.667 ms | 132 | 3 / 2,099 |
| Minimize controls, no capture | 612.678 ms | 33.334 ms | 12 | 4 / 2,106 |
| Minimize picture, capture controls | 574.712 ms | 6,816.688 ms (intentionally hidden picture) | 87 | 1 / 1,455 |
| Existing egui operator driver, scored cycles 2–3 | 561.895 ms | 50.000 ms | 4 | 4 / 1,262 |

The captured-controls trial has 88 committed frames without valid presentation evidence, including its one missing handler; captured-picture trial 135, no-capture operator trial 16, egui trial 8. All trials have zero content-link mismatches, missing completions, missing present calls, lost records, GPU errors or backpressure; max GPU work in flight is 1 and normal shutdown drains to 0. Unknown presentation outcomes must not be inferred from completion. In the captured-picture operator trial, 124 zero callbacks were inside the scored interval. Screenshot collection substantially perturbs this result (132 whole-process zeros versus 12 in the separate no-capture run); these are separate trials, not interchangeable performance samples.

**Controls minimized, picture observed:** 121 app-window screenshots over 28.534 s decoded to monotonically advancing native content IDs, from 343 to 1,992. No adjacent captured IDs repeated; every ID exists in an encoded content record. Maximum capture spacing was 289.006 ms. Inside the four measured controls callback gaps, sampled picture IDs advanced `497 → 511`, `841 → 855 → 869`, `1320 → 1334 → 1347`, and `1675 → 1689`. Valid native presentation counts during those gaps were 32, 33, 35 and 31. This is positive evidence of changing captured image content through controls transitions, with limited temporal resolution. It cannot exclude a shorter compositor-visible pause between captures or prove scanout.

**Picture minimized, controls observed:** 125 screenshots show four short repeated yellow-circle positions during transitions, spanning 236.566, 244.419, 237.853 and 470.634 ms between identical samples. The controls resume without restoring the hidden picture: each three-second confirmed-minimized hold contains 12 distinct circle positions. Controls CPU callbacks also recover. The picture's long presentation gap is expected while the picture itself is hidden; its content/presentation records resume on restore and shutdown succeeds. This reproduces the user's transition-only symptom in the yellow-circle window. It does **not** demonstrate a persistent GPU deadlock. The controls' main-thread/AppKit/eframe callback pause is observed; exact responsibility between synchronous native animation/event processing and drawable acquisition has not been newly isolated with a stack trace in this follow-up. The earlier shared-wgpu fence mechanism remains a separate baseline finding.

### Commands and evidence

```sh
cargo build --release --locked -p rastera-native-output
xcrun clang -fobjc-arc -framework Foundation -framework CoreGraphics \
  tools/native_window_info.m -o /tmp/rastera-window-info
python3 tools/native_output_probe.py native native --target=operator --capture \
  --cycles=2 --out=/tmp/rastera-content/operator.log
python3 tools/native_output_probe.py native native --target=picture --capture \
  --cycles=2 --out=/tmp/rastera-content/picture.log
python3 tools/native_output_probe.py native native --target=operator \
  --cycles=2 --out=/tmp/rastera-content/operator-no-capture.log
python3 tools/native_output_probe.py native egui --out=/tmp/rastera-content/content-egui.log
python3 tools/decode_native_content.py /tmp/rastera-content/operator.frames
python3 tools/summarize_native_output.py /tmp/rastera-content
```

The decoder uses installed `ffmpeg`/`ffprobe` and only supports this fixed 900×560 Retina picture screenshot layout. Screenshot filenames record request-start monotonic microseconds relative to driver launch, **not image acquisition/presentation timestamps**. Window-only capture may observe a surface differently from final desktop composition. Raw logs, driver event files and app-window PNGs are under `/tmp/rastera-content`; temporary evidence is not durable repository storage. Derived measurements and decoded IDs are preserved separately in `native-content-results.json`. Prior release comparison results remain unchanged.

Automated checks pass after instrumentation: formatting, locked workspace compilation, four Rust tests, Clippy with warnings denied, native release build with Objective-C++ warnings denied, and four focused Python analysis tests. The existing `block` future compatibility warning remains. No claim of fixing the baseline integrated output is made. Human retest is still required using the exact titles: watch the picture's frame digits/marker while minimizing/restoring `Rastera`, and watch the yellow circle and its visible timestamps while minimizing/restoring `Rastera — Native Output`. Keep acceptance pending; native capture, API timestamps and CPU diagnostics do not substitute for that observation.

### Timestamped retest of the freezing Metal controls

After adding the controls timestamps, a separate one-cycle picture-minimize/restore run captured 63 controls-window images (`picture-timestamps.log`). The timestamp-region pixels repeat across request-start samples at 8.268962 and 8.533436 s, a 264.474 ms span. Both show **elapsed 8.010 s, UI callback 464, Unix 1793211917576 ms**; see [first capture](evidence/controls-transition-a.png) and [later capture](evidence/controls-transition-b.png). Another repeated timestamp-region pair spans 230.315 ms during restoration. Sampling bounds the observed hold; it does not measure the exact onset/end of the freeze. The final timestamped build recovers after both transitions and exits normally. This directly records the user-identified window freezing, rather than relying on the other window's presentation counters.

Reproduce the timestamped direction with:

```sh
python3 tools/native_output_probe.py native native --target=picture --capture \
  --cycles=1 --out=/tmp/rastera-content/picture-timestamps.log
```

## Controlled animation-settings comparison

### Checkpoint, constants and settings restoration

Before changing the experiment, all existing native code, content instrumentation, timestamp captures and unresolved visual failure were committed and pushed as [`beeaec74adb7c853a3f75a650d55d33224f9fafa`](https://github.com/AlphaFatti/Rastera/commit/beeaec74adb7c853a3f75a650d55d33224f9fafa) on [`experiment/native-output-presenter`](https://github.com/AlphaFatti/Rastera/tree/experiment/native-output-presenter). This is a WIP checkpoint, not milestone acceptance. The animation experiment remains separate review changes after that checkpoint.

A single new release build was used for **all six runs**, SHA-256 `4cfb08a8351becd82d6f861c54481e9bac34d0f83951390d6ec2c2643dae394c` (unchanged before/after validation). Same Apple M1 Pro, macOS 15.5, Rust 1.99.1, eframe 0.31.1, wgpu 24.0.5, FIFO controls renderer, dedicated native Metal thread/queue, shader, work-in-flight bound and window layout. No dependency, renderer, rendering-thread, clock, App Nap setting or process activity assertion changes were made. Normal OS power-management behavior was left in place; it was not independently profiled. Runs were serial, in A-idle/A-cycles/B-idle/B-cycles/C-idle/C-cycles order. Each idle interval lasted 12 s; each cycles run performed three minimize/restore cycles with three-second **confirmed-state** holds.

| Condition | Native picture animationBehavior | Reduce Motion | Actual startup read-back |
| --- | --- | --- | --- |
| A | Default, property left untouched | Off | `requested=default readback=0 reduce_motion=0 main_thread=1` |
| B | `.none` on `Rastera — Native Output` only | Off | `requested=none readback=2 reduce_motion=0 main_thread=1` |
| C | Default, property left untouched | On | `requested=default readback=0 reduce_motion=1 main_thread=1` |

Temporary diagnostic option: `--window-animation=default|none`; default is unchanged AppKit behavior. The property is set and read on main before showing the native picture window, and logged with `NSWorkspace.accessibilityDisplayShouldReduceMotion`. Invalid option values fail before creating windows. The controls window's animation behavior is not changed. Apple's [animationBehavior documentation](https://developer.apple.com/documentation/appkit/nswindow/animationbehavior-swift.property) and installed SDK 15.5 `NSWindow.h` describe automatic **orderFront/orderOut** animations; they do not promise to suppress Dock miniaturization. This experiment tests that distinction rather than assuming that `.none` disables the native minimize path.

Original Reduce Motion was **off**: the normal **System Settings → Accessibility → Display → Reduce motion** checkbox returned `0` on this macOS 15.5 host. No preference files or `defaults` writes were used. After A/B, Accessibility clicked that same normal checkbox; read-back changed `0 → 1`, and both C app launches independently reported `reduce_motion=1`. A `finally` block restored the checkbox after C; read-back was `original=1 readback=0`. Thus the original **off** value is restored. No user intervention was needed. The current Apple Support UI may call the section [Motion](https://support.apple.com/guide/mac-help/customize-onscreen-motion-mchlc03f57a1/mac); the Display path here is the actual tested host UI.

### Observed image content and callback activity

Every minimize presses the picture window's native yellow `AXMinimizeButton`. Every restore uses its native `AXMinimized=false` Accessibility operation; the target state is polled and the separate controls window is checked unminimized. These are external native operations, not the egui command driver or order-out substitutes. All 18 requested transitions were confirmed and all six applications exited normally. Dock icon clicks and fresh human acceptance were not exercised by this automation.

Only the still-visible **yellow-circle `Rastera` window** was captured. Identical hashes of the elapsed timestamp/UI counter/Unix-time region establish repeated captured image content; the native picture's presentation counts are not used as evidence that the controls image changed. Each idle run has zero repeated sampled timestamp regions. Each cycles run has **six** repeated regions associated with the six long controls callback gaps, then resumes changing while the picture stays minimized or after restoration.

| Condition | Idle controls UI max gap | Cycles controls UI gaps (min–max) | Timestamp samples, idle / cycles | Repeated timestamp regions, idle / cycles | Screenshot hold lower bounds / upper bounds across six holds |
| --- | ---: | ---: | ---: | ---: | --- |
| A | 50.746 ms | 557.000–575.355 ms | 48 / 207 | 0 / 6 | 102–105 ms / 867–903 ms |
| B | 53.249 ms | 554.549–605.489 ms | 47 / 206 | 0 / 6 | 105–355 ms / 865–1,189 ms |
| C | 64.972 ms | 548.708–579.029 ms | 47 / 207 | 0 / 6 | 102–350 ms / 851–1,133 ms |

Screenshot bounds are intentionally loose, **not exact freeze durations or a ranking of settings**. The driver records both capture-command start and return for each PNG. Actual acquisition is somewhere inside that bracket. For identical first/last images, the conservative lower bound is `last.start − first.end` (clamped to zero). Different neighboring timestamp images bound the whole hold above by `next.end − previous.start`. Raw request-start spans are also retained. Capture request spacing reached 302 ms in the cycles runs (380 ms in A idle); shorter pauses can fall between samples. Capture includes process launch/Window Server overhead and perturbs scheduling. The approximately 0.55–0.61 s numbers above are **CPU callback gaps**, separately measured from monotonic callback entry times. They are not inferred from screenshots or presentedTime.

Representative identical-image pairs: [A first](evidence/animation-A-first.png) / [A last](evidence/animation-A-last.png), [B first](evidence/animation-B-first.png) / [B last](evidence/animation-B-last.png), [C first](evidence/animation-C-first.png) / [C last](evidence/animation-C-last.png). These preserve the longest acquisition-lower-bound pair in each condition, not a continuous video. A window-only capture can observe a surface differently from final desktop composition; no physical scanout verification is claimed.

### Native rendering and presentation, measured separately

The following measurements belong to the **picture window**, which is deliberately hidden for part of each cycles run. Its long whole-interval gaps must not be described as freezes of the visible controls. Positive `presentedTime` values are Apple-reported presentation evidence only. Zero timestamps, absent handlers and unknown outcomes remain in the accounting.

| Trial | Native schedule max gap | Valid picture presentation max gap | Valid presentation samples in scored interval | Zero timestamp callbacks, scored / whole process | Missing handlers / committed (whole process) | Committed without valid presentation evidence (whole process) |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| A idle | 35.470 ms | 41.667 ms | 645 | 50 / 61 | 4 / 1,136 | 65 |
| A cycles | 7,584.231 ms | 7,716.692 ms | 1,534 | 146 / 157 | 1 / 2,119 | 158 |
| B idle | 38.459 ms | 41.667 ms | 639 | 48 / 57 | 1 / 1,127 | 58 |
| B cycles | 7,684.757 ms | 7,816.690 ms | 1,536 | 141 / 154 | 3 / 2,119 | 157 |
| C idle | 50.024 ms | 58.333 ms | 610 | 69 / 81 | 4 / 1,128 | 85 |
| C cycles | 7,750.982 ms | 7,883.356 ms | 1,507 | 167 / 179 | 2 / 2,115 | 181 |

During **each** controls callback pause, the independent native path still delivered scheduling and command-completion callbacks: A 31–34 of each per gap, B 32–35, C 31–34. Valid native picture presentation samples within those same gaps numbered A 23–31, B 24–32, C 22–31. Surrounding valid-timestamp gaps were at most 33.334 ms (A), 41.667 ms (B/C). Zero-timestamp handler arrivals during individual controls pauses ranged from 4–6 (A), 4–8 (B), 4–7 (C); they are not positive presentation evidence. This separates stopped controls content generation from an application-wide GPU stop.

Across all six processes: no unavailable/nonfinite timestamp counter increments, missing completions, missing present calls, content/drawable-link mismatches, GPU errors, lost telemetry or semaphore backpressure. Maximum GPU work in flight was 2 in B-cycles and 1 otherwise, within the unchanged bound of 2. Each shutdown reported a complete drain and zero in flight. Missing presentation handlers are the final 1–4 sequences at shutdown; the GPU drain does not guarantee their delivery. Those frames and all zero-timestamp outcomes remain unknown rather than silently counted as displayed. Raw missing/invalid sequence lists are retained in [animation-settings-results.json](animation-settings-results.json).

### Where the image stops, and next measured fix candidate

**Neither `.none` nor Reduce Motion improved the confirmed transition-freeze behavior in this experiment.** All six visible content holds and callback pauses remain under both alternatives. Small maximum differences are not statistically meaningful: one short trial per condition, fixed trial order, uncontrolled thermal/compositor load, and screenshot perturbation. Reduce Motion is not proposed as a required user workaround, and `.none` is not a verified minimize-animation fix.

The first observable stop is the **main-thread controls update/paint path**. Its visible timestamp and UI frame identifier stop changing while callbacks are absent. The underlying `Instant` monotonic clock does not freeze: elapsed time jumps forward on the next update. The yellow-circle mesh and text are created in that update, so no new controls content is generated during the gap. The independent native renderer continues generating/completing work during the same gaps; no shared-canvas texture or UI-to-native uniform coupling exists in this diagnostic. This is not merely advancing presentation counters with a frozen native content ID, and no persistent GPU deadlock was demonstrated.

A separate, non-comparison run sampled the same binary with default behavior and Reduce Motion restored off (`sample PID 10 1`). The main-thread call tree includes:

```text
NSAccessibilityEntryPointPerformAction → NSCell accessibilityPerformPress
  → NSButtonCell performClick:
    → NSWMWindowCoordinator ... windowMiniaturizationResponse:
      → NSWindow _minimizeToDockBypassingWindowManager
        → NSWindow _regularMinimizeToDock
          → minimizeItems → waitForTransaction → CFRunLoopRunSpecific
```

`waitForTransaction` occurs in 473 sampled stacks beneath this minimize path; samples are **not exact duration measurements**. This supplies a specific AppKit miniaturization wait mechanism consistent with the UI callback/content pause. The same aggregate trace also spends much ordinary rendering time inside eframe/wgpu `CAMetalLayer.nextDrawable`; aggregate counts cannot assign all such waits to the transition, and this follow-up does not claim every pause has one exclusive cause. The sampled run had no screenshot capture, is outside A/B/C timing results, recovered, and exited normally. An excerpt and reproduction helper are preserved under `docs/evidence/animation-transition-stack.txt` and `tools/sample_native_transition.py`.

The next focused fix candidate is to remove the continuously moving preview/picture pixels' dependence on the controls' main-thread update/presentation path, reusing the already independent native presenter approach in a small measured preview experiment while keeping AppKit lifecycle and editor controls on main. Its test must again minimize the native picture and prove that both clocked content and presentation advance on the still-visible surface. This is a candidate for the next task, **not an implemented fix or a reason to accept this milestone**. Pumping/re-entering eframe manually inside AppKit's nested wait or moving AppKit window operations to a worker is not justified by this evidence. Integration with Rastera's shared canvas still requires a measured cross-queue texture ownership/synchronization design.

### Reproduction and checks

```sh
cargo build --release --locked -p rastera-native-output
xcrun clang -fobjc-arc -framework Foundation -framework CoreGraphics \
  tools/native_window_info.m -o /tmp/rastera-window-info
open 'x-apple.systempreferences:com.apple.Accessibility-Settings.extension?Display'
osascript tools/reduce_motion.applescript read  # Record original; tested value was 0.

# A: Reduce Motion off. B: repeat both commands with --window-animation=none.
python3 tools/native_output_probe.py native idle --target=picture --capture \
  --window-animation=default --seconds=12 --out=/tmp/rastera-animation/A-idle.log
python3 tools/native_output_probe.py native native --target=picture --capture \
  --window-animation=default --cycles=3 --out=/tmp/rastera-animation/A-cycles.log

# C: use the normal settings checkbox, then repeat A's commands with C filenames.
osascript tools/reduce_motion.applescript on
# Restore the originally recorded checkbox value after C, including on failure:
osascript tools/reduce_motion.applescript off  # This host's original value only.

python3 tools/summarize_native_output.py /tmp/rastera-animation
python3 tools/summarize_animation_captures.py /tmp/rastera-animation/A-cycles.frames
# Repeat capture analysis for each of the six *.frames directories.
```

For manual retest: `cargo run --release --locked -p rastera-native-output --bin native-output-control -- --window-animation=none` (or `default`). Minimize **`Rastera — Native Output`** and observe **`Rastera`**, then restore and repeat. The setting helper expects this host's English macOS 15.5 Display pane; if it is unavailable, use **System Settings → Accessibility → Display → Reduce motion** manually and restore the recorded original value. Do not change unrelated settings. The experiment's automated C runner used a `finally` block for restoration, and its read-back is preserved in the results JSON.

Passed after code changes: `cargo fmt --all -- --check`, `cargo check --workspace --locked --offline`, `cargo test --workspace --locked --offline` (four Rust tests), `cargo clippy --workspace --all-targets --locked --offline -- -D warnings`, native release compilation with Objective-C++ warnings denied, and seven Python analysis tests. New measurement tests check acquisition brackets, different-neighbor upper bounds and no fabricated hold from a single sample. Existing transitive `block` future-compatibility warning remains. Source/toolchain/dependency baselines are preserved. Raw logs, capture timing JSONL and app-only PNGs are temporary under `/tmp/rastera-animation`; derived records and representative evidence are in the repository. General continuity, integrated-output correctness and first-canvas acceptance remain pending human visual verification.
