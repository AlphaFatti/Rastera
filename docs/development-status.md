# Rastera development status

**Acceptance blocked: the native minimize/restore transition stall remains reproduced.**

This WIP checkpoint does not signify milestone acceptance. Individual native transitions were exercised; the stricter all-window automation check remains unverified. Automated checks pass, as detailed below.

Status recorded October 29, 2026. This is an engineering prototype, not a production live-show application. The architecture blueprint remains the long-term direction.

## Current canvas-mapping review work

The WIP `feature/canvas-mapping` remote-review checkpoint builds on editor checkpoint `7be441ed18e5bf97aa73fbda5052477bfc6407e9`. Run `cargo run --release --locked -p rastera-desktop --bin canvas-editor`. Settings adds undoable, transactional canvas dimensions; Canvas / Viewports modes add pan/zoom/rotation, persistent output names/colors and separate selections; installed font families/faces affect actual core text preparation. Version-1 projects migrate explicitly to schema 2. Central Rastera tokens and a neutral, always-visible operator-only canvas boundary are implemented. Logical outputs stay 1920 × 1080; default halves remain exact. Diagnostic baselines are preserved.

The producer remains independent, with completion-held native leases and atomic frame dimensions/matrices. Resize allocation is limited to two four-texture generations, each ≤256 MiB nominal RGBA texels; assets, drawables and driver overhead are additional. File/font/text preparation remains off the producer. [The current guide](canvas-mapping.md) documents ownership, limits, schema, controls and exact verification.

Twenty-four Rust tests, nine Python analysis tests, formatting, locked workspace compilation, denied-warning Clippy and the release build pass. Native replay covers odd portrait/landscape resize with outputs open, history, overlapping viewport selection/pan/zoom/rotation, fonts/fallback, save/reopen, both mapped outputs' close/reopen and failed candidate preservation. Actual mouse/keyboard controls accepted dimensions and Menlo Bold, confirmed in a scratch save and reopened for review. Empty black-canvas boundary capture shows all four edges.

Native three-cycle operator/Left/Right transitions recovered and drained with zero GPU errors/outstanding leases. Earlier background/captured trials include missing evidence and a multi-second clean-output gap. A final foreground clean-capture control reports both outputs advancing during the operator case (maximum positive API gap 37.501 ms); clean marker captures change across the trial. Sparse capture timing still cannot exclude a shorter visual pause. These results do not establish human continuity acceptance. The operator UI pause, capture-conditioned stalls, historical all-window check, human/external-display verification and overall milestone acceptance remain pending. Current results and static evidence are linked from the guide.

## Previous canvas-editor checkpoint

The WIP `feature/canvas-editor` review checkpoint builds on `9fbf7f81c2528ccf96143391adc0646bd9f6e976`. Run `cargo run --release --locked -p rastera-desktop --bin canvas-editor` on macOS 14+. It adds reusable image/text document objects, ordered layers, native preview selection/drag/corner resize, an inspector, editing/history, and versioned JSON projects with relative media references. File/image/text preparation is off the independent producer; complete scenes enter the existing four-texture, completion-leased native canvas path. Operator-only selection chrome never enters clean textures. Baseline executables are preserved.

[The editor record](canvas-editor.md) distinguishes pure automated checks, programmatic native event replay, captured content and remaining human input/display checks. Fifteen Rust tests, nine existing Python analysis tests, formatting, locked compilation, Clippy with denied warnings and the workspace release build pass. New focused tests cover Retina/letterboxing mapping, hit order/visibility/locking, anchored and aspect-locked resize, independent text style, undo/redo/IDs, PNG/JPEG/alpha, project references/validation and text wrapping/alignment/clipping. Font notices are included. No first-canvas or production acceptance is claimed; the operator UI pause, capture-conditioned callback stalls and historical stricter all-window automation gap remain pending.

Native review checks now include real PNG picker/text entry/inspector controls, image and text dragging/resizing across the seam, reorder, save/open panels and actual output checkboxes. The saved composition is open for human review. A final local-native-event replay additionally asserts whole-gesture history, negative positions, failed-load preservation and native output lifecycle. Corrected animated trials exercised three native minimize/restore cycles separately per operator/Left/Right, with captured and uncaptured runs. Clean marker captures change inside a 584.580 ms operator UI callback pause; 1:1 screenshot seam checks pass. GPU drains end with zero leases; no logged GPU errors or pool-full skips. Sparse captures, zero/missing presentation evidence, an earlier invalid static-motion trial and the unresolved operator/capture limitations are retained in the editor record/results. Human unobscured-display and physical-output acceptance remain pending.

The sections below retain the original diagnostic baseline's scope and historical verification; they are not the current editor feature inventory.

## Original diagnostic baseline: working scope

- Rust workspace with UI-independent geometry/GPU generation, the desktop shell, and a separate native presentation control crate.
- One fixed 3840 × 1080 offscreen GPU texture, generated via wgpu/Metal on a dedicated producer thread.
- Blue left/orange right diagnostic halves, identification blocks, 120-pixel grid, single-pixel seam columns, and animated markers.
- Full operator preview plus independently opened/closed native left and right projection windows. Exact 1920 × 1080 source halves; clean content with black aspect-fit bars and ordinary macOS window chrome.
- Shared texture, no CPU frame readback. Window resizing changes presentation only.
- Approximately 60 Hz monotonic producer deadlines, a wait for the producer's specific submission to bound producer work, and missed-deadline skipping. Deferred output viewports receive independent, coalesced frame-ready notifications. View presentation remains event-loop driven. See README for queue/presentation limitations.

## Original diagnostic baseline: verification record

Host: Apple M1 Pro, arm64, macOS 15.5; Apple Command Line Tools, without full Xcode. Rust 1.99.1 was installed in isolated `/tmp/rastera-cargo` and `/tmp/rastera-rustup` directories for this session. Those temporary directories are not a permanent developer setup; follow README for a normal installation.

Passed with the lockfile included in the review changes:

- `cargo fmt --all -- --check`
- `cargo check --workspace --locked --offline`
- `cargo test --workspace --locked --offline`: three tests pass. Two geometry tests cover complete slice partitioning, UV endpoints, aspect-fit letterboxing/pillarboxing, zero-size destinations, full-canvas mapping, and Retina scale equivalence; the desktop test checks bounded repaint notifications.
- `cargo clippy --workspace --all-targets --locked --offline -- -D warnings`
- `cargo build --workspace --locked --offline`
- `cargo tree -i wgpu`: one shared wgpu 24.0.5 version across core, desktop, eframe, and egui-wgpu.

Native application launched with `./target/debug/rastera-desktop --outputs`. Runtime adapter report: **Apple M1 Pro, Metal**. The process initialized and ran without a logged shader/GPU validation error during the smoke check. This confirms native startup, not a human visual acceptance test or a sustained-performance measurement.

Cargo reports a future Rust compatibility warning in transitive `block` 0.1.6 (an uninhabited extern static, Rust issue 74840). Current checks pass; evaluate dependency upgrades before moving beyond this experiment. eframe 0.31.1 and wgpu 24.0.5 are an intentionally compatible older pair; Rust and dependencies are pinned for reproducibility.

## Human visual checks still required

1. Run `cargo run --locked -p rastera-desktop -- --outputs`. Arrange the three windows so they are visible.
2. Confirm full blue/orange canvas in the operator, blue only in Left, orange only in Right. Left has one identification block; Right has two. Markers should move continuously in all views.
3. Resize each window wide, tall, and small. Check centered black bars, undistorted grid/markers, and unchanged source content. Try moving outputs between Retina and external displays.
4. At a 1:1 physical-pixel output size, inspect green pixel 1919 at the left output's right edge and magenta pixel 1920 at the right output's left edge. Check for duplicated/missing seam content. Arbitrary window scales cannot prove exact physical seam alignment.
5. Close/reopen each projection with its checkbox. Confirm clean outputs contain no operator controls. Close the operator to quit.
6. Check minimize/restore and display changes; record freezes or pacing issues. No behavior under App Nap, sleep, long shows, or display hot-plug has been certified.

## Next milestone and limitations

Next: resolve and visually accept the minimize/restore regression before accepting the first-canvas milestone. Then verify geometry on two real displays, add presentation timing/late-frame instrumentation and a short endurance measurement, and complete human editor verification while preserving the diagnostic baseline. The separate libobs feasibility comparison in the blueprint has not been performed.

There is no guaranteed cross-window frame identity at presentation time, genlock, rational media timeline, calibrated color pipeline, display assignment/fullscreen mode, editable routing, capture, video decoding, audio, NDI, DeckLink or slides. The later editor adds image/text composition and save/load. Windows is not implemented or tested. Minimum supported macOS, installer/signing, source license, and performance limits remain undecided. No mock integrations are present.

## Minimize/restore regression investigation

User observation: animation in all three windows pauses during minimization and resumes when the transition ends. Other behavior tested by the user appeared to work. This is a transition stall, not evidence of a persistent deadlock. **Do not accept the first-canvas milestone yet.**

### Findings and changes

Read the exact installed sources for egui/eframe/egui-wgpu 0.31.1, egui-winit 0.31.1, and wgpu/wgpu-core 24.0.5 (wgpu-hal 24.0.4) before changing scheduling:

- `egui/src/context.rs`: immediate viewports couple parent/child repaints; deferred callbacks can run while their parent sleeps. Replaced immediate projections with deferred viewports. Closing a projection signals the operator through an atomic flag.
- `wgpu-types/src/lib.rs`: `Maintain::Wait` targets the latest shared-queue submission. Changed to `WaitForSubmissionIndex` for the producer's own submission. This removes an unnecessarily broad dependency but does not isolate the device from presentation.
- `wgpu-core/src/present.rs` and `device/queue.rs`: acquiring a drawable holds a device fence read lock, while queue submission requires its write lock. A slow drawable acquisition can therefore stall producer submission too. A native stack sample found the UI thread in `CAMetalLayer::nextDrawable`. The phase counters also captured producer submission pauses. This identifies a mechanism for the shared stall; it does not prove every transition pause has the same stack.
- `egui-wgpu/src/winit.rs`: drawable acquisition occurs on the UI/event thread before the paint pass; independent callbacks do not give the windows independent presentation threads.
- `egui-winit/src/lib.rs`: on macOS the minimized flag is not refreshed every frame (documented deadlock workaround). Native Accessibility commands confirmed minimized windows while egui's cached flags still reported false. Do not use that cached flag to decide whether a window can safely draw or resume.

Completed canvas frames now wake each enabled viewport independently, with at most one pending producer repaint request per viewport. Closed outputs receive no frame notifications. A new test simulates 10,000 frame notifications while a viewport does not run, verifies a single pending repaint cause, and checks that notifications work again after it resumes. The canvas texture and slice geometry are unchanged.

Added opt-in `--diagnostics` logging from an independent monitor thread: producer submitted/completed counters, encoding/submission/wait phase, maximum submission/wait time, each viewport's UI entry/exit and GPU paint-pass count, latest producer completion observed during paint, and maximum UI callback gap. Paint-pass entry is not a presentation timestamp, and the observed producer count is not a guaranteed displayed frame ID.

An experimental Immediate presentation mode did not eliminate transition gaps and was reverted; the candidate retains FIFO. No dependency versions were changed. The current correction removes repaint and GPU-wait coupling but must not be described as a verified elimination of the native transition freeze.

### Regression verification

Automated formatting, locked compilation/build, all three tests (two existing geometry tests plus bounded repaint notifications), and Clippy with `-D warnings` pass. The existing transitive `block` future-compatibility warning remains.

The opt-in `--minimize-test=operator|left|right|all` driver ran each case in a separate native process, three minimize/restore cycles per case. All processes exited normally, views resumed, and sampled producer submissions/completions differed by at most one. Viewport paint callbacks advanced after restoration. Transition gaps remained, including with deferred viewports alone and experimental Immediate mode. These runs send synchronous native window commands through egui and are not human visual acceptance.

After Accessibility was granted to VS Code, external System Events tests controlled only the owned process's windows through `AXMinimized` and read back each target state. Baseline runs exercised each case independently, three cycles per case; each restore succeeded. This also showed the stale egui minimized-state issue described above. Candidate native-control runs attempted two cycles per case. Immediate Accessibility readbacks sometimes preceded the asynchronous state change, so those readbacks alone are not acceptance evidence. Follow-up tests poll until the requested native state is confirmed. The candidate still exhibits transition stalls: the confirmed operator run measured approximately 575–591 ms maximum UI callback gaps across all three views and a 441 ms maximum producer submission call, versus a 4 ms maximum GPU-completion wait. The confirmed Left run likewise measured roughly 589–605 ms UI gaps while producer submission/wait maxima were only 10/3 ms. This separates UI/presentation interruption from stopped GPU generation. Counters recover after restoration. **The transition freeze has not been eliminated.**

### Required manual retest

Run `cargo run --locked -p rastera-desktop -- --outputs --diagnostics` and test separately:

1. Minimize the operator with both projections visible; observe both during the transition and while the operator remains minimized.
2. Minimize Left while operator/Right remain visible; repeat separately for Right.
3. Restore each window and confirm immediate recovery without restarting.
4. Minimize all, restore them, and repeat the cycles. Check for progressive slowdown, stuck windows, and gaps in the diagnostic counters.

Record whether any pause occurs only during the macOS animation or persists while a window stays minimized. Visual acceptance remains pending even when counters and native state readbacks succeed. If transition gaps persist, further presentation-path work is required; independent egui callbacks alone cannot guarantee continuous presentation when the shared UI thread or shared-device drawable acquisition blocks.

Follow-up native-state polling confirmed minimize and restore for Operator, Left, and Right individually, with all views resuming. The stricter all-window follow-up timed out after 8 seconds in System Events while confirming Right's minimized state; the app's independent diagnostics continued to show increasing producer, UI, and paint counters until the owned test process was stopped. This is an automation timeout, not proof of an app deadlock, and that follow-up's all-window restoration is **unverified**. Earlier three-cycle native-command driver runs completed all-window restoration, but they do not replace this check or human observation. No first-canvas visual acceptance is claimed.


## Controlled presentation investigation

See [presentation-investigation.md](presentation-investigation.md) for the experiment based on WIP commit `3aa156d7811eafe8c5ffaa591f976ba83ba4fbcc`, using unchanged toolchain/dependency versions and the debug profile. A headless core producer and a minimal deferred three-window framework control now isolate the two paths. Corrected interval diagnostics split encoder finish from queue submission, completion wait and frame-ready notification, with monotonic timestamps and per-viewport CPU activity.

The framework-only control reproduces roughly 0.6-second projection callback gaps through both native button invocation and the egui driver. The full native-button trial has similar UI gaps while producer cadence continues near target; egui-driven restore additionally stalls the producer's queue submission. This is reproduction/association evidence, not actual display-presentation timing or proof of the precise OS/framework lock involved. Four automated tests now pass, together with formatting, all-target compilation, build and Clippy. The earlier WIP's “submit” maxima included encoder finalization and cannot distinguish those two calls.

This investigation exercises operator transitions only. It does not verify the historical stricter all-window automation case, replace human visual acceptance, fix the stall, or change the presentation backend. Milestone acceptance remains pending.


## Native output presentation control

The controlled presentation investigation was checkpointed and pushed as `2a9238e9e26636e95e021301e2e3a86975e30d2c`. The separate `experiment/native-output-presenter` branch adds a macOS 14+ diagnostic executable with an eframe operator and an independent CAMetalDisplayLink/native Metal output. Baseline renderer/application/control code and existing dependency versions remain unchanged.

[Native output investigation](native-output-investigation.md) records matched release results. Metal reported 33–34 output presentations within each approximately 553–575 ms operator UI callback gap; the maximum gap between valid reported presentation timestamps in the scored native-transition trial was 41.667 ms. Zero presentation timestamps and shorter pacing disturbances remain. This is API presentation evidence, not physical scanout verification or proof that Rastera's integrated shared-canvas output is fixed. Resize, output minimize/restore and both close orders were exercised with clean GPU drains. Four Rust tests, three Python analysis tests, formatting, compilation and Clippy pass. Overall first-canvas milestone acceptance remains pending.

### Native control image-content follow-up

The separate native experiment checkpoint `d1f3b56` was pushed before further changes. Both exact window directions are now recorded in [the native investigation](native-output-investigation.md). Minimizing `Rastera — Native Output` reproduces a transition-only pause in `Rastera` (the eframe/wgpu Metal yellow-circle controls); captured elapsed timestamps and UI callback IDs stop, then recover while the picture stays minimized. No persistent GPU deadlock was demonstrated. In the reverse direction the native picture's captured, rendering-thread-generated frame IDs advance through controls callback pauses. Capture resolution and compositor/scanout limitations remain explicit. Added visible timestamps to the freezing controls and native drawable/content-ID joins; no integrated output or presentation-backend fix is claimed. Automated checks pass; human visual acceptance and the overall milestone remain pending.

### Animation-settings control

The instrumentation/confirmed visual failure was checkpointed and pushed as `beeaec7`. One identical release binary compared default animations/Reduce Motion off, `.none`/off on the native picture, and default/on, each with captured idle and three native minimize/restore cycles. All conditions retain six visible controls timestamp holds and roughly 0.55–0.61 s UI callback gaps. Neither setting fixes the reported direction. Reduce Motion was changed through normal System Settings and restored to its recorded original off value. Independent native scheduling/completion continues during the controls gaps; a separate stack sample finds AppKit's minimize-to-Dock transaction wait on main. [The investigation](native-output-investigation.md) preserves loose screenshot acquisition bounds, zero/missing presentation evidence and limits. Four Rust and seven Python tests, formatting, locked checks, Clippy and native release build pass. The next preview isolation candidate is documented but not implemented. Acceptance remains pending.


## Native shared-canvas integration — review work

Animation-settings WIP checkpoint `fe359157ba2bd6c41dfc013b082b2969f9344eaa` was pushed before these changes. The separate `shared-canvas-native` desktop executable now uses the actual core generator with one producer and a bounded four-texture, GPU-completion-leased handoff. Native full preview and Left/Right sample the same logical canvas; clean outputs have exact existing slice geometry and no UI overlays. Baseline render paths remain available, with native-control titles clarified.

[The integration record](shared-canvas-integration.md) distinguishes API checks, captured moving content and human/hardware acceptance. Staged consumers, three-cycle operator/Left/Right transitions, clean-only/no-capture controls, 1:1 screenshot seam checks, resizing, native close/UI reopen and normal zero-lease shutdown were exercised. Captured visible output markers advance inside a 576 ms UI callback pause. Capturing the composited operator still causes unresolved native callback stalls; cached window snapshots can also contradict presentation counters. Those limits remain explicit. Default macOS settings and dependency versions are preserved. Six Rust tests, nine Python tests, formatting, compilation, release build and Clippy pass. Operator UI pauses, broader capture continuity, physical-display verification and overall acceptance remain pending.
