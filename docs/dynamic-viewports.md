# Dynamic viewports — review milestone

October 29, 2026. `feature/dynamic-viewports`, based on `fix/viewport-selection` at `4ee19523a890808e8a80a897205e4500884a1067`. First-canvas acceptance remains pending. This changes logical mapping and lifecycle; it does not fix the operator UI transition pause or add fullscreen/display assignment.

## Run and controls

```sh
cargo build --release --workspace --locked
cargo run --release --locked -p rastera-desktop --bin canvas-editor
# Seven portable preset definitions; explicitly open the first four at startup:
cargo run --release --locked -p rastera-desktop --bin canvas-editor -- --project=examples/dynamic-viewports.rastera.json --outputs=4
```

**Live Edit — changes affect outputs.** In **Viewports** mode, Add Viewport creates a uniquely named, colored, centered HD definition at 100% canvas footprint scale (1:1). It does not open a window. Remove Selected deletes its definition and retires any associated consumer. Open clean output is a separate checkbox for each ID; closing a window preserves the definition. Zero viewports is valid and leaves the canvas editor/preview operational. Names and IDs identify footprints, list rows and clean window titles.

The practical limit is **eight logical viewports**, regardless of open-window count. An attempted ninth produces a clear error. This bounds editor/native bookkeeping; it is not a performance guarantee for eight outputs. IDs are never positional indices. The allocator advances through deletion and undo branches; undo/redo restores definitions, IDs and settings, but does not reopen windows. Loading a project retains the session allocator high-water mark. Saving persists the current allocator. Output windows and future display destinations are separate from logical identity; no physical-display routing is implemented.

Width/Height are drafts until **Apply dimensions**, with presets for 1920×1080, 1920×1200, 1024×768, 1080×1920, 1080×1080, 3440×1440 and 960×192. Custom axes are 1–8192, with at most 16,777,216 logical pixels and no aspect restriction. The inspector shows the applied ratio. Apply is undoable and preserves center, zoom and rotation; unchanged zoom means changed dimensions change the sampled footprint. Fit Canvas fits the complete canvas. 1:1 canvas pixels resets footprint scale to 100% without changing center, rotation or output dimensions. Canvas footprint scale is `100 / zoom` percent; corner dragging changes this scale while preserving output resolution. Zoom supports 1/8192–8192.

Matrices, inverse mapping, footprints, orientation arrows, handles and native sampling use each viewport's dimensions. Pixel bounds remain half-open with a top-left origin. Default projects retain the exact original 1920×1080 Left/Right split. Clean windows open and respond to changed output dimensions at 1:1 backing-pixel size using AppKit coordinate conversion on the main thread. Native windows still aspect-fit without stretching when manually resized; this does not change logical dimensions or canvas geometry. Out-of-canvas samples are black.

Overlap priority remains selected handles, any name/nearby screen-space outline, selected body, then other bodies. Option-click cycles overlapping IDs deterministically, selecting only; the next ordinary drag moves the chosen viewport. Selection adds no document history. Gestures latch IDs and create one history entry. Sidebar selection still works. Workspace remains `#17151A`; editing chrome never enters shared clean textures.

## Persistence and ownership

Schema **3** stores a variable list, per-viewport `size` and `next_viewport_id`. Explicit v1 migration restores the original canvas/default halves; v2 migration preserves viewport IDs, names, colors and transforms and assigns each the original 1920×1080 dimensions. The allocator starts above migrated IDs. A schema-3 empty list remains empty; it is not replaced with defaults. Invalid counts, duplicate/zero IDs, sizes, transforms or allocator state reject the candidate without replacing the current document. Image/text/reference handling is unchanged. Save before opening another project or quitting; no recovery/unsaved-close prompt exists.

The producer stays independent of eframe callbacks and surface acquisition. File/image/font/text preparation stays on the existing worker; bounded latest-state publication delivers complete prepared scenes at frame boundaries. Mapping-only updates prepare immutable metadata there, rather than allocating viewport tables on the producer.

Each completed frame owns an `Arc<[FrameViewport]>` containing IDs, logical dimensions and matrices. A native lease keeps both that table and the shared Metal texture alive. The ABI passes a borrowed pointer/count, valid until lease release. Rust and Objective-C++ assert 48-byte frame headers, 48-byte viewport rows, table-pointer offset 32 and matrix offset 16. The native encoder looks up its own ID in its leased frame, copies that row into uniforms, and explicitly skips/releases a frame whose ID is missing. There is no array-position fallback. Dimension and mapping changes cannot mix data from different frames.

One producer, four textures per generation, at most two generations, completion-held leases, validated same-native-device sharing and no CPU frame readback remain. Each native consumer has two GPU-work slots. Removing/closing stops its display link/thread and drains its GPU work; completion callbacks own leases independently of the logical document/window. Unaffected consumers remain keyed by ID. Reopening creates a fresh consumer instance for the same logical ID. Diagnostics include viewport ID and ephemeral consumer-instance ID so reopenings cannot be mistaken for remapping.

The opt-in `--dynamic-probe` additionally submits one private-buffer GPU texel read behind a 50 ms shared-event gate, then deletes that viewport while its work is pending. This is diagnostic-only, not part of normal rendering. Its completion releases the same source lease/group/semaphore as normal GPU work; it performs no CPU readback or presentation. Those events are reported separately from drawable presentation. GPU timestamps can be unavailable/zero.

## Verification record

Automated checks passed on this Apple silicon Mac:

```sh
cargo fmt --all -- --check
cargo check --workspace --locked
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo build --release --workspace --locked
python3 -m unittest discover -s tools -p 'test_*.py'
```

There are 34 Rust tests and nine Python analysis tests. The existing pinned `block 0.1.6` dependency emits a future-incompatibility notice; toolchain and dependency versions remain unchanged. New core regression coverage includes zero/one/many definitions, limits, allocator/history branching and redo, custom-aspect transforms/inverses/fit/hit testing, rotated/coincident cycling, deletion before a later ID, immutable leased metadata, v1/v2 migration, every preset's project round trip and saved empty lists. Existing pixel-exact split, overlap priority, pointer mapping, gesture/history and editor tests remain.

Native release replay (no global mouse/keyboard input):

```sh
# Helpers used by the PID-scoped driver:
clang tools/native_window_info.m -framework Cocoa -framework CoreGraphics -o /tmp/rastera-window-info
clang tools/native_editor_window.m -framework Cocoa -framework ApplicationServices -o /tmp/rastera-editor-window
python3 tools/dynamic_viewport_probe.py --out /tmp/rastera-dynamic-pending-read --foreground --capture
python3 tools/canvas_editor_probe.py --out /tmp/rastera-dynamic-legacy --selection --cycles=0 --no-capture
```

The dynamic replay passed all 33 phases: remove every definition, reopen empty project, add/open four differently proportioned outputs (HD, portrait, XGA, banner), change portrait to WUXGA and undo, remove the earliest active ID, undo/redo/undo and reopen it, close/reopen another, add a fifth ultrawide output while others remain open, remove/undo/change it to square, cycle five overlapping IDs, drag only the chosen ID and undo, save/reopen, reach the eight-definition limit, exercise an undo branch and verify increasing IDs, remove all consumers, reopen empty, add one, then reload the saved composition. Metadata matches the document by ID; unaffected consumer-instance IDs remain unchanged. The separate legacy replay passed image/text editing, save/load failure preservation, nested labels/outlines, sidebar dragging, Option modifiers/cycling and baseline resize/rotation.

An initial trial sampled zero GPU work at removal, so it did not establish in-flight retirement. The gated-read trial sampled **one pending GPU read** on ID 3. Commit callback-host time was 424470.301971458; completion callback-host time 424470.353004667; reported GPU end 424470.352847375. These are not presentation times. Both runs exited normally; final texture occupancy was **0/4**, with no reported GPU errors or drain timeout. Focus was restored. Normal macOS settings, toolchain/dependencies and rendering threads were preserved.

[Recorded per-ID/per-instance events](dynamic-viewport-results.json) preserve zero timestamps and unmatched logged drawable commits. In the captured dynamic trial, all logged presentation callbacks reported zero presentation timestamps; several retired instances had commits without a logged presentation callback. These do not establish physical drops or visible continuity. The GPU-only read requests no presentation callback and is counted separately. No API count is treated as proof that content changed on screen. A final uncaptured replay of the final release binary (`/tmp/rastera-dynamic-redo`) also passed all phases, sampled one pending GPU read and drained to 0/4 slots.

Owned clean-window captures under `/tmp/rastera-dynamic-pending-read` show diagnostic content fitted inside the different window proportions and no editing overlays; their grid scale is consistent with the mapping. These scratch captures/logs are not portable committed evidence. The probe uses local native events/editor actions and PID-scoped window controls; it does not replace human interaction with the new Add/Remove/dimension controls. It does not retest minimize continuity, sustained eight-output load or external displays.

The seven-preset project was launched for human review with four clean windows arranged beside the operator. For a fresh review session, use the seven-preset command above. Add/remove/open outputs independently; resize and rotate overlapping footprints, apply custom dimensions, undo/redo, save/reopen and check identities/mappings. Confirm ordinary window resizing never stretches the picture. Observe clean motion while minimizing/restoring the operator and another output. The existing operator pause, capture-conditioned stalls, zero/missing presentation evidence and stricter historical all-window automation limitation remain unresolved. Human unobscured-display/physical-output acceptance and overall milestone acceptance remain pending.

## Pixel-sizing correction after checkpoint 9669600

The prior defaults fitted new viewports to the complete canvas, and open native windows retained their original small frame when logical dimensions changed. That obscured the relationship between resolution, footprint and window size. New definitions and the seven-preset example now start at unit zoom. Existing saved transforms are preserved; use **1:1 canvas pixels** for their footprint, or adjust **Canvas footprint scale** deliberately. For example, 1920×1080 at 100% covers 1920×1080 canvas pixels; at 200% it covers 3840×2160 canvas pixels but still outputs 1920×1080 pixels. The operator canvas preview itself remains scaled to fit the editor.

Clean windows now use [AppKit backing-coordinate conversion](https://developer.apple.com/documentation/appkit/nswindow/convertfrombacking(_:)?language=objc) rather than a fixed point size. Opening, reopening or applying dimensions requests that many backing pixels; undo/redo resizing follows completed leased-frame metadata by ID. A manual window resize remains an aspect-fitted preview until the next dimension change or reopen. No CPU pixel readback, producer change or new dependency is involved. Physical scanout is not verified by backing-store dimensions. Very large/tiny windows and display-mode limits can constrain usable desktop geometry; these cases and moving between different-scale monitors remain unverified.

Automated checks: formatting, locked workspace check, **36 Rust tests**, nine Python tests, denied-warning Clippy and release workspace build pass. New tests cover pixel-exact footprints for all seven presets and independent footprint scaling. The native release command `python3 tools/dynamic_viewport_probe.py --out /tmp/rastera-pixel-sizing --foreground` passed all lifecycle phases plus initial drawable-size assertions and live resize/undo assertions. `shared_window_size` reports exact backing sizes for all seven presets, including 1920×1080 → 960×540 points and 1080×1920 → 540×960 points on this Retina screen; WUXGA Apply reports 1920×1200 backing pixels. It exited 0, restored focus and drained to zero outstanding slots. This was programmatic native verification without captures, not human visual or physical-output acceptance. Historical pause/capture limitations remain pending.
