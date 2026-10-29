# Shared canvas / native clean outputs — WIP

The animation-settings experiment was checkpointed and pushed as [`fe359157ba2bd6c41dfc013b082b2969f9344eaa`](https://github.com/AlphaFatti/Rastera/commit/fe359157ba2bd6c41dfc013b082b2969f9344eaa) on `experiment/native-output-presenter` before this integration. Its failure, screenshots and limitations remain recorded in [the native investigation](native-output-investigation.md). The integration is a WIP checkpoint for remote review on the same experiment branch. **Neither production readiness nor resolution of the operator UI pause is claimed.**

## Executables and exact roles

| Executable | Window titles | Content |
| --- | --- | --- |
| `shared-canvas-native` | `Rastera — Operator / UI preview` | eframe controls above a native, full shared-canvas preview |
| Same executable | `Rastera — Native clean output — Left` / `Rastera — Native clean output — Right` | Actual core canvas halves, without editor overlays |
| `native-output-control` | `Rastera — Operator / UI preview (control)` / `Rastera — Native clean output (control)` | Preserved independent yellow-circle/native-pattern presentation control; **not shared-canvas integration** |
| `rastera-desktop` / `framework-control` | Existing `Rastera`, `Rastera — Left`, `Rastera — Right` baseline titles | Original shared-wgpu application / framework-only control respectively |

Native-control selectors and its stack helper now use the explicit `(control)` titles. The new driver uses exact role titles plus the PID of the process it launched; it never selects a window by color, title substring, frontmost app or ambiguous “Metal” terminology. Historical measurements retain the titles that existed when recorded. Original baseline render/scheduling behavior is preserved; the core generator setup/encoding was factored into `PatternGenerator` so both paths use the same shader.

## Implementation and GPU ownership

`rastera-core` owns geometry, the unchanged diagnostic pattern and the new completed-frame pool. Desktop/AppKit ownership lives in `rastera-desktop`. This is **one logical 3840 × 1080 canvas, one producer thread/clock**, with four GPU texture slots, not four independent generators. Source rectangles remain half-open, top-left-origin `Slice::FULL`, `Slice::LEFT` `(0,0,1920,1080)` and `Slice::RIGHT` `(1920,0,1920,1080)`.

The new executable creates a headless Metal-only wgpu device/queue independently of eframe's device, queue, surfaces and repaint callbacks. The producer uses the existing WGSL generator, monotonic time, approximately 60 Hz deadlines and missed-deadline skipping. Native consumers have their own Metal queues, drawable pools and CAMetalDisplayLink run-loop threads. They sample the producer's actual `MTLTexture`; they do not generate replacement patterns. The full native preview is an AppKit child view below the controls, accounting for winit's flipped view coordinates. UI text remains eframe-driven and can pause during native window transitions. Clean outputs do not depend on those UI updates.

Ownership/synchronization is deliberately conservative:

1. A mutex-protected pool has four `Arc<Texture>` slots and one latest-frame `(slot, frame_id)` mailbox. The producer reserves only a non-current slot whose sole reference is the pool. It never overwrites the current frame, a reserved writer, or a leased reader.
2. The producer writes the time uniform, encodes the existing shader, submits on its own queue and calls `device.poll(WaitForSubmissionIndex(own_submission))`. Only after GPU completion does it publish the slot and a monotonically increasing core frame ID under the mailbox mutex. No egui notification is involved.
3. Each native display-link callback acquires a lease on the latest completed slot. The private lease owns its texture through all native GPU reads. The unsafe wgpu 24.0.5 `Texture::as_hal<Metal>` / wgpu-hal 24.0.4 `raw_handle` export borrows that texture; neither native code nor consumer code manually destroys it.
4. The native queue and CAMetalLayer use **the exact producer texture's `MTLDevice`**, not an assumed system-default device. Initialization validates RGBA8Unorm and 3840 × 1080. Every frame also validates device object identity, registry ID and slice bounds. All tested roles reported registry ID `4294969684` (Apple M1 Pro), compatible dimensions and exact producer-device selection. Other GPUs/interoperability paths are not certified.
5. A native render pass reads the leased canvas texture and writes its own drawable. Aspect-fit rectangles come from the actual `rastera-core::fit` function via FFI; the fragment shader uses integer nearest pixel reads inside the selected slice. Window/drawable resize changes mapping only. After encoding it registers completion/presentation handlers, commits, then calls ordinary drawable `present()`, following the CAMetalDisplayLink contract.
6. The command-completed handler releases that frame lease **after GPU reads finish**, including error completion. Presented handlers retain only telemetry and copied identifiers, not a canvas lease. An output drawable contains a rendered snapshot; retaining it for display does not require retaining the canvas slot. A slot becomes reusable only after it is no longer current and **every** consumer GPU lease has gone. Thus producer writes and native reads never overlap on that slot, even across different queues. This uses CPU-observed GPU completion as the ordering boundary; it does not assume that different queues automatically synchronize resources.
7. Each native consumer permits at most two GPU commands in flight; busy callbacks skip rather than queue. If all pool slots are busy, the producer skips that deadline instead of allocating more textures or waiting on a slow viewport. No per-window FIFO, CPU frame readback, IOSurface copy or CPU fallback is present.

References consulted against the installed APIs: [wgpu 24.0.5 texture export](https://docs.rs/wgpu/24.0.5/wgpu/struct.Texture.html#method.as_hal), [wgpu-hal 24.0.4 raw texture](https://docs.rs/wgpu-hal/24.0.4/wgpu_hal/metal/struct.Texture.html#method.raw_handle), [Metal completion handlers](https://developer.apple.com/documentation/metal/mtlcommandbuffer/addcompletedhandler(_:)), and [CAMetalDisplayLink delegate ordering](https://developer.apple.com/documentation/quartzcore/cametaldisplaylinkdelegate/metaldisplaylink(_:needsupdate:)). Completion handlers establish finished GPU work, **not display presentation**.

The fixed canvas allocation is 66,355,200 bytes (63.28 MiB) for four RGBA8 textures. Each consumer additionally has a three-drawable pool and a two-command bound. Native telemetry is bounded to 4,096 POD events per live consumer. These are application bounds, not a claim about all OS/driver memory or long-term leaks.

AppKit window/view creation, resize/backing updates, close and detachment stay on main. Closing an output stops and joins its display-link thread and drains GPU work with a two-second timeout. Rust frees its source context afterward. Late GPU completion handlers own their texture leases/semaphore/group and never access the freed context or AppKit. The operator stops the reporter, stops all consumers, then joins the producer. All successful runs ended with `shared_shutdown outstanding_slots=0`. A GPU/driver hang, indefinite framework call or drain timeout is not certified recovery; leases must remain retained rather than being reused prematurely.

`shared_event` records distinguish event/handler CPU time (`callback_host_s`) from GPU end time on `kind=complete` and drawable `presentedTime` on `kind=presented` (`gpu_or_presented_host_s`). Commit event time is CPU commit-return observation, not presentation. Records carry role, callback sequence, drawable ID and the exact leased core frame ID. Zero/nonfinite presentation values and missing handlers are not replaced with arrival times. Consumer sequences restart on reopen; analysis joins role/sequence/core-frame identity for these trials. The producer's interval measurements separately time cadence, recording, encoder finish, queue submit and completion wait.

## Build and run

macOS 14+ and Apple Command Line Tools are required for this executable. This Mac: Apple M1 Pro, macOS 15.5, SDK 15.5, Apple clang 17, Rust 1.99.1. No full Xcode installation was needed. Installed Apple frameworks and system C++ are used by the Objective-C++ bridge. Direct dependencies `foreign-types = 0.5.0`, `raw-window-handle = 0.6.2` and `pollster = 0.4.0` were already in Cargo.lock transitively; only desktop dependency edges were added. Existing versions, including eframe 0.31.1 / wgpu 24.0.5 / wgpu-hal 24.0.4, are unchanged. Mach-O inspection retains minimum OS 11.0 for the baseline desktop binary and 14.0 for this new executable; these are link settings, not platform certification.

```sh
cargo build --release --workspace --bins --locked
cargo run --release --locked -p rastera-desktop --bin shared-canvas-native

# Staged bring-up: one native full-canvas consumer first, then add Left, then both.
cargo run --release --locked -p rastera-desktop --bin shared-canvas-native -- --outputs=0
cargo run --release --locked -p rastera-desktop --bin shared-canvas-native -- --outputs=1
cargo run --release --locked -p rastera-desktop --bin shared-canvas-native -- --outputs=2

# Preserved controls / original application:
cargo run --release --locked -p rastera-native-output --bin native-output-control
cargo run --release --locked -p rastera-desktop --bin framework-control
cargo run --release --locked -p rastera-desktop -- --outputs
```

The operator checkboxes close/reopen the native clean windows; native close buttons clear the checkbox. Closing the operator exits. Arrange all windows without overlap before human continuity checks. Default macOS animations, Reduce Motion (off as previously restored), and App Nap behavior were preserved throughout this task. No activity assertions or global preferences were changed.

## Actual verification and remaining capture limitation

All integration trials below used the same release executable, SHA-256 `9b4270804deeb691934952ac4db0e405b79c3ed900276421d54401f1ab21b7dc`. The driver arranges the operator at `(20,44)`, size `1000×430`, and clean windows at `(20,520)` / `(520,520)`, each `480×300` points. A separate bounds read-back after minimize/restore confirmed both clean windows remained onscreen, with unchanged nonoverlapping bounds. Native yellow minimize buttons were pressed through Accessibility; restore used native `AXMinimized=false`. These are real native operations by automation, not human acceptance or Dock-click verification.

Staged runs brought up the native full preview alone, then preview + Left, then preview + Left/Right. The captured full preview contains the complete blue/orange core canvas; Left contains blue/one white block and Right orange/two blocks. All processes initialized on Metal and closed normally. See [operator capture](evidence/shared-canvas-operator.png).

| Continuity trial | Three cycles? | Still-visible clean output | Max valid Metal presentation gap | Captured image evidence |
| --- | --- | --- | ---: | --- |
| Minimize operator, visible desktop content regions | Yes | Left and Right | 83.334 ms each | 194 samples each; no repeated detected marker positions; one Right sample lacked a detectable marker and is not positive evidence |
| Minimize Left, no screenshots | Yes | Right | 41.667 ms | API/core-frame evidence only |
| Minimize Right, no screenshots | Yes | Left | 41.667 ms | API/core-frame evidence only |
| Minimize Left, clean-output-only window capture | Yes | Right | 58.334 ms | 185 samples, marker detected/changed in every sample |
| Minimize Right, clean-output-only window capture | Yes | Left | 58.334 ms | 187 samples, marker detected/changed in every sample |

Both clean output markers demonstrably changed in captures acquired wholly within the **same 576.463 ms operator UI callback gap**. Left's thumbnail marker position advanced `105.5 → 121.5`; Right's `112.5 → 121.5`. Their capture command start/return brackets lie entirely inside that gap. The Left consumer committed 30 distinct source frames (IDs 508–542) and had 28 positive presentation reports in that gap. See [Left a](evidence/shared-visible-left-a.png) / [Left b](evidence/shared-visible-left-b.png), [Right a](evidence/shared-visible-right-a.png) / [Right b](evidence/shared-visible-right-b.png). Source frame identity is recorded metadata; the screenshots prove moving pattern content, not an exact displayed frame-ID match.

This does not establish perfect cadence. The screenshot runs have roughly 0.2-second acquisition spacing, occasionally up to 481 ms in the clean-only Left trial. The visible-region run's maximum spacing was 284 ms (Left) / 260 ms (Right). Acquisition lies between command start and return, not exactly at its filename/request time. Shorter freezes or missed refreshes can escape sampling; no physical scanout, camera, photodiode, projector or genlock verification was performed. The single missing-marker region sample remains unclassified, not silently counted as animation.

Two important adverse results are preserved:

- Initial output-minimize runs also captured the composited operator window. The still-visible native consumers then had scheduling/presentation gaps up to **1.954 s** (minimize Left) and **4.579 s** (minimize Right). The producer continued (maximum measured cadence gap 54.758 ms), the pool never filled, and the affected consumers had no GPU work in flight. Their callback counters stopped, rather than queueing more rendering work. This was absent in the separate no-capture and clean-only reruns. Capturing the composited operator is therefore an **unresolved continuity limitation associated with capture**, not a verified-safe operation or proof of a GPU deadlock. Its precise compositor/display-link cause remains unproven.
- The initial operator-minimize run's window-image captures also returned repeated marker snapshots while native scheduling and positive presentation reports continued (maximum valid gap 58.334 ms). A separate visible-content-region run captured advancing pixels through all three cycles. Window-image snapshots cannot be treated as interchangeable with the visible composed desktop. This discrepancy is retained, not discarded in favor of counters. Exact same-event simultaneous capture-method comparison and operator screen-recording/streaming behavior remain unverified.

The normal-path shared GPU handoff/device checks did not expose an integration blocker on this Mac. The capture-conditioned stalls above remain a blocker to a broader continuity/recording claim and must be investigated separately before accepting that behavior. **The operator's UI callback pause remains separately reproduced** (598.265 ms maximum in the visible-region operator run; roughly 0.54–0.62 s in clean-only transition runs). An independent native preview is not a fix for eframe control text/input pausing inside AppKit transitions.

### Geometry, resource bounds and lifecycle

At a 2x backing scale, each output was resized to `960×568` points including chrome. Captures were `1920×1136` pixels, leaving a `1920×1080` content area. At the middle row, Left's last column alone was green; Right's first column alone was magenta, with its next column the expected grid and subsequent column orange. Adjacent checked columns did not duplicate the seam. Captured display-profile RGB values are not raw canvas RGBA values. This checks the integer mapping on this host, not two physical projector edges. See [Left seam](evidence/shared-canvas-left-seam.png) and [Right seam](evidence/shared-canvas-right-seam.png); existing complete-partition/aspect-fit tests also pass.

Both outputs were independently resized through `700×360`, `350×500`, `480×300` native window sizes, closed by their native buttons and reopened through the actual operator checkboxes without restarting or replacing the producer. The reopen driver first needed correction: Accessibility's generic click and a first mouse click failed to toggle the inactive egui window. An explicit title-bar focus click followed by the checkbox click succeeded for both outputs. Failed driver logs remain in `/tmp/rastera-shared/failed-ax-click` and `failed-cg-click`; those terminated processes are not counted as graceful shutdown tests.

Across successful integration trials: no logged GPU validation/consumer errors, lost native telemetry, missing GPU-completion records, or pool-full skips. Sampled leased slots stayed at most 2 of the fixed 4; each consumer's observed maximum GPU commands in flight was 1, below its enforced limit of 2. Every successful run closed normally with complete native GPU drains and zero outstanding canvas slots. This checks bounded ownership and normal cleanup, not forced saturation, GPU-hang recovery, exhaustive leak detection or an endurance test. Positive/zero presentation timestamps, unpresented outcomes and missing-handler sequence lists are retained in [shared-canvas-results.json](shared-canvas-results.json); late/absent presentation handlers are never inferred from completed GPU reads.

Baseline native-control, original full-wgpu application and framework control each launched and shut down normally in separate release smoke runs after the change. The native-control title selectors matched. These are startup/close checks, not a new visual acceptance or sustained-performance comparison.

## Reproduce measurements

Accessibility and Screen Recording must be granted to the launching app/terminal. Helpers scope captures/clicks to the process launched by the driver; visible-region mode crops only clean-output interiors. Other windows/notifications may occlude a region, so keep the test desktop clear.

```sh
xcrun clang -fobjc-arc -framework Foundation -framework CoreGraphics \
  tools/native_window_info.m -o /tmp/rastera-window-info
xcrun clang -fobjc-arc -framework Foundation -framework CoreGraphics \
  tools/native_click.m -o /tmp/rastera-native-click

python3 tools/shared_canvas_probe.py preview --out=/tmp/rastera-shared/preview.log
python3 tools/shared_canvas_probe.py single --out=/tmp/rastera-shared/single.log
python3 tools/shared_canvas_probe.py idle --out=/tmp/rastera-shared/idle.log
python3 tools/shared_canvas_probe.py operator --cycles=3 --region --out=/tmp/rastera-shared/region/operator.log
python3 tools/shared_canvas_probe.py left --cycles=3 --no-capture --out=/tmp/rastera-shared/no-capture/left.log
python3 tools/shared_canvas_probe.py right --cycles=3 --no-capture --out=/tmp/rastera-shared/no-capture/right.log
python3 tools/shared_canvas_probe.py left --cycles=3 --clean-only --out=/tmp/rastera-shared/clean-only/left.log
python3 tools/shared_canvas_probe.py right --cycles=3 --clean-only --out=/tmp/rastera-shared/clean-only/right.log
python3 tools/shared_canvas_probe.py geometry --out=/tmp/rastera-shared/geometry.log
python3 tools/shared_canvas_probe.py lifecycle --out=/tmp/rastera-shared/lifecycle.log

# Adverse capture condition, intentionally retained for investigation:
python3 tools/shared_canvas_probe.py left --cycles=3 --out=/tmp/rastera-shared/left.log
python3 tools/shared_canvas_probe.py right --cycles=3 --out=/tmp/rastera-shared/right.log
python3 tools/summarize_shared_canvas.py /tmp/rastera-shared --images
python3 tools/summarize_shared_canvas.py /tmp/rastera-shared/clean-only --images
python3 tools/summarize_shared_canvas.py /tmp/rastera-shared/region --images
```

Image analysis requires `ffmpeg` and `ffprobe` on PATH; it measures yellow marker positions separately from GPU records. Raw logs/capture acquisition JSONL/PNGs are temporary under `/tmp/rastera-shared`. Derived summaries, bounds read-backs and representative captures are retained in the repository. Summary UI-gap maxima include only callbacks exceeding the explicit 100 ms logging threshold; zero means no such logged gap, **not zero latency**.

Passed: `cargo fmt --all -- --check`, `cargo check --workspace --locked --offline`, `cargo test --workspace --locked --offline` (six Rust tests), `cargo clippy --workspace --all-targets --locked --offline -- -D warnings`, release workspace/binary build, Objective-C++ warnings-as-errors compilation, and nine Python tests. New Rust tests retain two readers of the same slot until both finish and exercise pool exhaustion/release/reuse. Analysis tests keep GPU completion separate from presentation, preserve zeros/missing callbacks and use actual presentation timestamps rather than handler arrival. Cargo.lock is included; the existing transitive `block` future-compatibility warning remains.

Still required: human observation of both clean outputs during operator/other-output transitions; external-display/physical seam verification; Dock restores; display hot-plug/backing-scale moves; minimize-all/App Nap/sleep; longer load/endurance and saturation tests; and resolution of operator-capture callback stalls. Cross-output simultaneous frame identity, calibrated color, fullscreen routing, Windows, media/audio/NDI/DeckLink/slides and production readiness remain outside this prototype. **First-canvas acceptance remains pending.**

## Editor reuse

The subsequent [canvas editor](canvas-editor.md), based on integration commit `9fbf7f81c2528ccf96143391adc0646bd9f6e976`, reuses this producer/pool/native ownership path. Immutable prepared image/text scenes enter at frame boundaries, and only the editor preview gains native input/selection chrome. This diagnostic executable and earlier controls remain available. Editor verification does not supersede the operator/capture limitations or human/hardware acceptance requirements above.
