# Nested viewport selection correction

Current dynamic counts, dimensions, schema and verification supersede the fixed-two-output scope below: see [dynamic viewports](dynamic-viewports.md). Earlier measurements and limitations are retained.

October 29, 2026; `fix/viewport-selection`, based on `d4ce7f7d5ffe6b4e58424d7a0ffe196370712456`. Focused correction for remote review; first-canvas and human visual acceptance remain pending.

## Behavior

The former selected-body-first rule let a large selected Right footprint steal clicks from Left's visible label and border. Labels were drawn but had no independent hit target.

Ordinary clicks now resolve in this order:

1. Selected viewport's resize/rotation handles.
2. Labels across all viewports, then nearby footprint outlines.
3. Selected viewport's body.
4. Other bodies, in reverse paint order.

Label hit boxes use the native renderer's actual 12-point semibold font measurements and the same six-point anchor offset. Outlines use Euclidean distance to rotated screen-space segments, with a six-point tolerance independent of zoom/Retina scale. Handles retain a nine-point hit radius. When labels coincide or outline distances tie, the selected viewport wins, then reverse paint order; distinct nearer outlines win over farther ones.

Option-click overrides dragging and cycles hit viewport IDs in ascending order, wrapping from the current selection. It does not start a gesture or add document history. The next ordinary body/name/outline drag moves the chosen viewport; handles still resize/rotate. A sidebar hint exposes this workflow. A drag retains its original viewport index until completion, even when crossing another footprint or changing modifiers. Sidebar selection uses the same finish-gesture/inspector path as before. A completed change remains one undo entry.

The Objective-C++/Rust pointer ABI now includes device-independent native modifier flags (`u64`), with matching 32-byte layout assertions. Native diagnostic replay accepts the same flags and creates real local NSEvents with them. The Option bit was checked against the installed AppKit `NSEvent.h` (`1 << 19`). Label measurement and AppKit view/event work stay on the main thread. Hit geometry is reusable pure core code; interaction/history stays in desktop. Producer, texture generation, frame metadata, native consumer synchronization and scene schema are unchanged.

Workspace changes from the preceding `#19151F` to **`#17151A`**: `[0x17, 0x15, 0x1A, 255]`. Existing shared-token use updates operator panels/dialogs, native margins and label backgrounds. The accent palette and project-content colors are unchanged.

## Run/reproduce

```sh
cargo run --release --locked -p rastera-desktop --bin canvas-editor -- --project=examples/nested-viewports.rastera.json
```

This media-free fixture uses Left center `(1400,540)`, zoom 2 and Right center `(1400,540)`, zoom 0.75, so Right surrounds Left. Choose **Viewports**. Select Right in the sidebar, then drag Left's visible name or edge; Right must stay unchanged. Undo, move Right independently, and try a sidebar-selected Left body drag. Option-click inside both footprints to cycle; then release Option and drag normally. To test perfect coincidence, give both viewports the same center/zoom/rotation. Check resize and round rotation handles normally.

```sh
cargo fmt --all -- --check
cargo check --workspace --locked
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo build --release --workspace --locked
python3 -m unittest discover -s tools -p 'test_*.py'

# Local native view-event replay; existing helper setup is in canvas-mapping.md.
python3 tools/canvas_editor_probe.py --selection --out /tmp/rastera-selection-fix --cycles 0 --no-capture
```

## Actual verification

- **Automated:** all required checks and release build passed; 28 Rust tests (27 core, one desktop) and nine Python analysis tests. New cases cover nested/partial/coincident/rotated footprints; measured label/outline priority; five-point hits versus eight-point misses at 1×/2×/3× scales; stable-ID cycling; selected-body/sidebar priority and selected handles. The native flag ABI compiles with matching assertions. Dependency versions and Cargo.lock are unchanged. Existing `block 0.1.6` future-compatibility notice remains.
- **Native Mac:** release replay passed on the Apple silicon Mac. It dispatches local AppKit NSEvents to the actual preview, rather than injecting core pointer structures or global mouse input. With Right surrounding Left, it selected Left via its label and outline, moved it and checked the entire Right definition stayed equal. It then moved Right with Left unchanged. Undo restored the complete document; the label gesture had exactly one history entry. Sidebar-selected dragging remained latched across the other body with Option pressed mid-drag. Coincident Option-clicks reached Rust through the native bridge, cycled/wrapped, ignored movement while held, created no history, and allowed the next ordinary drag to move only the chosen viewport. Existing image/text/mapping replay, including ordinary viewport resize/rotation, also passed. Final shutdown reported `outstanding_slots=0`, with no logged GPU errors.
- **Launch:** the release editor was launched with the portable nested fixture for human review. Choose Viewports to see the footprints. Existing editor sessions are preserved.
- **Not claimed:** no new human physical mouse/keyboard acceptance, external-display/physical scanout checks or presentation continuity certification. This run did not repeat minimize/restore/capture timing experiments (`--cycles 0 --no-capture`). The operator transition pause, capture-induced stalls/stale snapshots, zero/missing presentation data and pending overall acceptance remain as documented in canvas-mapping.md.

The test artifacts/logs remain under `/tmp/rastera-selection-fix`; the reproducible project is committed under `examples/`. No system animation settings, App Nap policy or rendering threads were changed.
