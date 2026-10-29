# Assigned-display outputs — review milestone

October 29, 2026. `feature/display-outputs`, based on `e05ea35775d9b71482393d184d80bad4971961b2`. This adds native display destinations; first-canvas/production acceptance and existing pause/capture limitations remain pending.

## Run, assign and recover

```sh
cargo build --release --workspace --locked
cargo run --release --locked -p rastera-desktop --bin canvas-editor -- --project=examples/dynamic-viewports.rastera.json --outputs=0
```

In **Viewports**, select **Windowed** or **Fullscreen**, choose a target for fullscreen, then **Enable output**. Destination controls are separate from output resolution and canvas footprint scale/center/rotation. Each logical viewport retains its persistent ID. Status reports Disabled, Starting, Active, Missing display, Ambiguous display, Conflicting assignment or Error. One active fullscreen viewport may own each display. An existing unchanged owner wins a conflict; another feed is never silently moved or replaced. A rejected request is disabled and must be explicitly enabled after correction. Windowed outputs can coexist.

**Disable All Outputs · ⌘⇧Esc** is always in the operator toolbar, including Canvas mode. **Command–Shift–Escape** is application-scoped and works independently of egui text-field focus through a native local-event monitor. It immediately hides clean windows, then retires their consumers on the next app update. If another app has focus, Command–Tab back to Rastera first. Assigning fullscreen to the operator display deliberately covers the controls; this shortcut is the recovery path. Command–Q also quits Rastera. Recovery does not change preferred assignments.

**Identify Displays** shows click-through labels for about three seconds, then removes them. Labels match display name, backing dimensions, CG number and UUID prefix in the list; hover a target for its complete UUID and desktop origin. These labels are separate AppKit windows and never enter the shared canvas. CG numbers identify current labels only; saved assignments use Core Graphics display UUIDs. UUID availability/stability depends on the monitor/adapter/OS. Missing or duplicate UUID matches require reassignment; no ordinal, name or nearest-screen fallback exists. Mirrored panels are not independently addressable desktop destinations.

The editor starts with outputs disabled unless an explicit `--outputs=N` requests windowed startup outputs. Opening/reopening a project disables all current outputs before its candidate scene can reach the producer, including when loading ultimately fails. Saved fullscreen preferences are never automatically enabled, even with `--outputs=N`. Re-enable intentionally after opening a show.

## Native behavior and ownership

Fullscreen is a **borderless NSWindow at the selected NSScreen.frame**, not `toggleFullScreen` or a new macOS fullscreen Space. AppKit screen coordinates are used directly, including negative origins. Drawable dimensions come from the view's backing conversion; the native shader aspect-fits the logical viewport without distortion. Output resolution, footprint scale, center and rotation are unchanged. Resolution Apply/undo does not resize a fullscreen window: `syncWindowSize` runs only for Windowed mode.

Clean windows cannot become key/main windows. Fullscreen is click-through, shadowless, at `NSStatusWindowLevel + 1` (26 on this Mac), with `hidesOnDeactivate=NO`, `canHide=NO`, and CanJoinAllSpaces / CanJoinAllApplications / Stationary collection behavior. It is intended to stay above ordinary application windows/menu/Dock during switching, including on the operator screen. This does not bypass system security UI, lock screens or sleep. It does not alter global presentation settings, hide the cursor, change display modes or take exclusive display capture. Stage Manager, Mission Control and other apps' native fullscreen Spaces still require human verification.

Returning to Windowed restores its prior frame when reachable. If logical dimensions changed in fullscreen, current resolution determines the restored backing-pixel size; if unchanged, previous manual window geometry is retained. Unreachable geometry is repositioned. Windowed pixel sizing keeps the title bar and, when it fits, the picture on a visible desktop. Manual resizing still aspect-fits without changing document sampling.

Screen-parameter notifications reposition existing fullscreen windows or close consumers whose UUID no longer resolves. This main-thread path operates without waiting for an egui repaint. UI discovery refreshes about once a second and on Refresh. Missing assignments are preserved and never automatically resumed after reconnection. A changed/unresolvable UUID needs explicit reassignment. Unaffected consumers keep their IDs and destinations.

All AppKit discovery, identification, lifecycle and placement run on main. Existing CAMetalDisplayLink consumer threads continue rendering. Fullscreen startup defers showing the window until placement, avoiding a windowed flash on the wrong screen. Mode/reassignment updates the existing window/layer; deletion, disabling, missing displays and project changes join/drain consumers. GPU completion retains source leases and immutable per-frame ID/dimension/matrix metadata. The shared independent producer, four textures per generation, at most two generations, per-consumer two-work limit, exact source Metal device validation and no CPU canvas readback remain. Physical-display drawables/driver allocations are additional to the canvas-pool budget.

## Project and local settings migration

Portable project schema **4** adds only a random 128-bit `project_id`. Explicit v1/v2/v3 migration preserves previous canvas, layers, text/media, viewport IDs/names/colors/dimensions/transforms and allocator behavior (v1/v2 retain original HD output dimensions). Save a migrated project once to retain its generated identity across reopenings. Existing schema-4 identity round-trips unchanged; invalid identities reject without replacing the document. The preset demo is now schema 4.

Preferences live in `~/Library/Application Support/Rastera/output-destinations.json`, schema 1, separately from the portable project. Keys contain the machine's IOPlatformUUID, project identity and canonical project path (encoded without lossy filename conversion), then stable viewport ID. Unrelated projects with identical viewport IDs and copied files at different paths cannot inherit assignments. Save As intentionally carries current preferences into the new path; subsequent files are independent. Unsaved projects have session-only preferences. Enabled state is never serialized. Deleted viewport preferences remain available for logical undo; undo never reopens a window.

Local writes use a bounded two-MiB file and temporary-file rename. Invalid/future settings remain untouched and report an error. If machine identity cannot be obtained, preferences cannot be persisted. File operations stay off the producer. `getrandom 0.4.3` and `serde 1.0.229` are newly direct dependencies already present at those exact versions in Cargo.lock; no resolved versions/toolchain changed. The native bridge additionally links Apple's IOKit framework for machine identity.

## Verification

Required formatting, locked workspace check/tests, denied-warning Clippy and release workspace build pass. There are **36 core tests**, **four destination tests built into each of the two native desktop binaries**, **one desktop scheduling test** (45 successful Rust test executions), and nine existing Python analysis tests. Tests cover missing/ambiguous resolution, conflict ownership, project/machine/path isolation, disabled-on-reopen persistence, protected invalid settings, v1/v2/v3 migration, schema-4 round trips and existing geometry/lease/history invariants. Existing dynamic and editor/mapping/selection native replays also pass; they now explicitly re-enable windowed outputs after project reloads, matching the new safety policy.

```sh
cargo fmt --all -- --check
cargo check --workspace --locked
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo build --release --workspace --locked
python3 -m unittest discover -s tools -p 'test_*.py'

# Native PID-scoped helpers, if not already built:
clang tools/native_window_info.m -framework Cocoa -framework CoreGraphics -o /tmp/rastera-window-info
clang tools/native_editor_window.m -framework Cocoa -framework ApplicationServices -o /tmp/rastera-editor-window
# These briefly cover connected displays, activate only the test app and restore focus:
python3 tools/display_output_probe.py --out /tmp/rastera-display-final
python3 tools/display_output_probe.py --out /tmp/rastera-display-captured --capture
# Optional offline marker analysis requires installed ffmpeg; the app does not use it:
python3 tools/display_output_summary.py --markers /tmp/rastera-display-final /tmp/rastera-display-captured
python3 tools/dynamic_viewport_probe.py --out /tmp/rastera-display-dynamic --foreground
python3 tools/canvas_editor_probe.py --out /tmp/rastera-display-legacy --selection --cycles=0 --no-capture
```

[Recorded native/API/capture results](display-output-results.json) describe matched release trials on this Mac:

| Check | Actual result | Evidence limit |
| --- | --- | --- |
| Connected targets | Two screens; one OS-reported external | Not an independent physical-cable inventory |
| Retina target | Frame `(0,0,1800,1169)` points; drawable `3600×2338` | Backing pixels, not physical scanout verification |
| External target | Frame `(1800,89,1920,1080)` points; drawable `1920×1080` | Human panel observation pending |
| Reassignment / simultaneous outputs | Correct UUID targets and independent full-frame windows on both screens, at 2× and 1× backing scale | Only one external display available |
| Resolution / lifecycle | Fullscreen Apply/undo kept full frame; Windowed restored `1920×1080`; deletion, undo, close/reopen, project reopen and shutdown passed | Programmatic native assertions |
| Focus / recovery | Operator retained key focus; actual local ⌘⇧Esc event disabled feeds while fullscreen covered its display; Identify labels expired | Not human keyboard acceptance |
| Three minimize/restore cycles | Actual AX minimize-button presses and restore; external fullscreen remained reported onscreen | Window visibility is not image-motion proof |
| App switching | Fullscreen remained reported onscreen while another app was active | Ordinary switching only; broader Spaces/Stage Manager pending |
| Shutdown | Both matched trials exited 0 with 0/4 outstanding texture slots; no logged GPU error/drain timeout | Finite trials, not sustained reliability |

The final external consumer's maximum **positive API presentation gap** was 50.0004 ms in both uncaptured and captured trials. Its records include 12/18 zero presentation timestamps respectively, preserved rather than excluded from the report. Retired consumers have unmatched logged drawable commits; these do not establish physical drops. Whole-run maximum logged operator CPU callback gaps (>100 ms logging threshold) were 1022.025 ms / 1010.989 ms, including deliberate mode/retirement operations. Callback arrival timestamps are not presentation times. No new deadlock was observed in these trials, and no general pause fix is claimed.

The five owned clean captures show a full clean external picture without editor overlays. Yellow-marker centers advance across idle, three minimize samples and application switching: **932.5, 1061.5, 1511.5, 281.5, 1018.5 pixels**. Capture acquisitions took about 115.5–156.9 ms; request/return bounds are retained. These sparse samples cannot exclude a shorter freeze or define an exact freeze duration. Capture can perturb playback; the uncaptured control is retained. No human-visible interruption duration was measured. Earlier bridge-discovery encoding and inactive-probe focus setup failures were corrected before these final trials; their failed runs do not count as acceptance.

**Pending:** human confirmation of the physical target, complete panel coverage, aspect ratio, editing, visible minimize/restore continuity and recovery; actual cable disconnect/reconnect; negative-origin physical arrangement; two or more external displays; sustained load; Stage Manager/Mission Control/native fullscreen Spaces, sleep/lock and display-mode changes. No macOS settings or physical connections were changed. Existing operator transition/capture limitations and overall milestone acceptance remain pending. A review build is launched with outputs disabled; enable the external target deliberately in Viewports.

Implementation references: Apple's [Spaces collection behaviors](https://developer.apple.com/documentation/appkit/nswindow/collectionbehavior-swift.struct/canjoinallspaces), [joining other applications](https://developer.apple.com/documentation/appkit/nswindow/collectionbehavior-swift.struct/canjoinallapplications), [screen-parameter notifications](https://developer.apple.com/documentation/appkit/nsapplication/didchangescreenparametersnotification), and installed macOS 15.5 SDK declarations for Core Graphics display UUIDs/AppKit backing conversions. These API policies are not substitutes for the pending human checks.
