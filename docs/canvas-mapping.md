# Canvas dimensions, output mapping and fonts — review milestone

October 29, 2026. WIP remote-review checkpoint on `feature/canvas-mapping`, based on canvas-editor checkpoint `7be441ed18e5bf97aa73fbda5052477bfc6407e9`. Diagnostic executables remain available. This expands the editor; it does not accept first-canvas, fix the operator transition pause, or establish production readiness.

## Run

```sh
cargo build --release --workspace --locked
cargo run --release --locked -p rastera-desktop --bin canvas-editor
# The existing version-1 demo migrates on load:
cargo run --release --locked -p rastera-desktop --bin canvas-editor -- --project=examples/canvas-demo.rastera.json
```

The review session is open with `/tmp/rastera-mapping-ui.rastera.json`, saved through the real native panel during this test. It has a 2559 × 1439 canvas and Menlo Bold text. This scratch file references the repository's synthetic example image; it is not a portable packaged project. Use the example command above on another machine.

**Live Edit — changes affect outputs.** Save before opening another project or quitting; there is still no unsaved-close prompt or recovery file.

See [the focused overlap-selection correction and verification](viewport-selection-fix.md) for the changes following checkpoint `d4ce7f7`. Earlier trial data below is retained.

## Editor controls

- **Settings → Canvas settings:** presets or numeric width/height, Apply/Cancel. Layers retain their positions and sizes in canvas pixels. Viewport centers, zoom and rotation retain their coordinates. Content outside the new canvas clips. Apply prepares a candidate while the old composition continues; the editor commits after a completed frame is published. Undo/redo includes canvas changes.
- **Canvas / Viewports:** segmented toolbar modes with separate object/viewport selections. Switching finishes pending gestures/inspector edits. Object duplicate/delete shortcuts apply only in Canvas mode and surrender to text fields. Viewport gestures each record one completed history entry.
- **Viewports:** select a labelled footprint or list row, drag a visible name or outline to select/pan it even inside another selected footprint; otherwise the selected body takes priority. Option-click cycles overlapping viewports in stable ID order without dragging; release Option, then drag the chosen body. Drag a corner toward/away from its center for uniform zoom, or drag the round rotation handle. The arrow marks the top of the sampling frame. The inspector edits name, RGBA identity color, center X/Y, zoom and clockwise rotation. Names/colors also appear in the toolbar/list/inspector; native window titles follow names. IDs remain document data.
- **Fit Canvas:** centers on the current canvas, sets rotation to zero and chooses the uniform zoom that contains the whole canvas, leaving black where necessary. **Reset:** restores the original half's center `(960,540)` or `(2880,540)`, zoom 1 and rotation 0, retaining ID/name/color even on a custom canvas.
- **Fonts:** select an editable text layer, choose **Choose font family / face…**, search family or style, then choose a face. The separate native dialog lists installed faces alongside bundled defaults. Requested family/style/PostScript identity persists; missing/unusable faces retain that reference and display a bundled-fallback warning in the inspector.

The current workspace/panel/dialog/preview-margin token is **`#17151A`** (`[0x17, 0x15, 0x1A, 255]`); accents and project content colors are unchanged.

The always-visible boundary is light neutral `#DAD5DE`, 1 screen point, with 20 screen points of workspace margin. Object handles, labels, viewport footprints and orientation indicators belong to the AppKit operator overlay. They never enter the shared texture. Preview layers clip to their layout bounds, including objects extending into workspace margins. Root UI and the native overlay use centralized Rastera palette tokens; project content colors remain independent.

Settings uses an AppKit menu; Settings and Fonts use separate native eframe windows above the AppKit preview. Their input does not depend on drawing egui popups over a native child view. These prototype dialogs use an always-on-top window level and disable background editor interaction while open.

## Core geometry and schema

`rastera-core::mapping::Viewport` is the single transform model for footprints, inverse hit testing and clean-output sampling. Both outputs remain logically **1920 × 1080** regardless of native window size or Retina scale. For logical output position `q`, source position is:

```text
source = center + clockwise_rotation(rotation) * (q - [960,540]) / zoom
```

Canvas coordinates start at the top left. Zoom 1 means one canvas pixel per logical output pixel; higher zoom samples a smaller region. Sampling uses nearest pixel reads and explicit black outside the source. Each completed frame carries canvas dimensions and both affine matrices atomically with its texture lease. Native consumers validate texture/device/dimension compatibility and use those matrices. Window resize changes only the aspect-fit destination rectangle.

Default projects retain exactly the original halves: Left pixels `[0,1920) × [0,1080)`, Right `[1920,3840) × [0,1080)`. Mint Left (`#26D085`) and Yellow Right (`#FFFE00`) have stable IDs 1 and 2. A full pixel-center test verifies every default slice pixel. The diagnostic background scales to custom canvases; its original single-pixel seam reference is exact only at 3840 × 1080.

Project schema **2** stores dimensions, ordered image/text objects and layers, two viewport definitions, and optional font references. Loading schema 1 explicitly migrates its fixed 3840 × 1080 geometry to schema 2 with original output transforms and bundled default text fonts. Existing examples remain schema 1 to exercise that path. Unknown versions and malformed/over-budget documents fail without replacing the current composition. Saved projects use schema 2; older binaries cannot read them. Relative image paths still resolve against the project location. Window-open state and undo history are not project data.

## Preparation, ownership and budgets

There is still one UI-independent producer, targeting 60 Hz with a monotonic clock and no catch-up queue. Its clock and native consumers do not depend on eframe repaint callbacks or surface acquisition. Source writes finish on the GPU before frame publication. Native Metal consumers retain leases until their own GPU completion handlers run, independently of drawable presentation callbacks.

Preparation uses the existing bounded latest-document request and one ready-scene slot. Allocation, decoding, font discovery/loading and text layout happen off the producer. Immutable cached text tiles include the font reference in their cache keys. Core Text discovers installed font files on a worker, using its [documented thread-safe font APIs](https://developer.apple.com/documentation/CoreText); `ttf-parser` finds the requested face, including legacy ASCII PostScript names in Mac collections. Font bytes are cached up to 64 MiB, with a 32 MiB per-file limit; projects never embed installed font bytes. Unavailable/unreadable/unsupported files use the existing bundled Ubuntu Light and fallback definitions. Font discovery refreshes on application restart. Color/variable fonts and complex-script typography are not certified.

Canvas validation uses checked arithmetic, the actual device dimension limit and these prototype limits:

- Axes: 1–min(device limit, 8192) pixels.
- Four RGBA8 textures per generation: at most 256 MiB nominal texels (`width × height × 16`), or 16 Mi-pixels.
- At most two live generations/eight canvas textures: at most 512 MiB nominal texels. Asset tiles, native drawables, font/preparation memory and Metal allocation overhead are additional; this is not a total-process GPU-memory guarantee.

A candidate size gets a complete new four-texture generation on the preparation worker. Validation/OOM scopes cover canvas and scene-resource preparation. Failure keeps the old live document/output. A third generation waits until predecessor references expire; allocation does not grow a resize queue. The producer switches only to a complete prepared scene, uses a free non-current slot and atomically publishes new dimensions/matrices after completion. Old leases remain valid through pool replacement. Both the pool and its generation retain textures; free-slot accounting distinguishes those baseline owners from GPU-reader references. Old resources disappear only after all owners/readers release them. No CPU canvas readback was added.

Actual native runs logged at most two generations, four textures per generation, no missing GPU completion records, zero GPU errors/lost telemetry, and zero outstanding slots on shutdown. Those are bounded-work observations, not an endurance or peak total-memory certification. Real GPU OOM/device loss was not injected; failed media preparation and invalid-size recovery were exercised.

Dependencies/toolchain versions are unchanged. `ttf-parser 0.25.1` and desktop `serde_json 1.0.154` now have direct dependency edges; both were already locked transitively. The native bridge additionally links the system CoreText framework. Bundled font notices remain in [THIRD_PARTY_NOTICES.md](../THIRD_PARTY_NOTICES.md).

## Verification record

Host: Apple M1 Pro, arm64, macOS 15.5 (24F74), release profile for native runs. System animation/Reduce Motion settings were unchanged; the existing native executable’s App Nap policy and rendering threads were preserved.

| Verification | Actual result |
|---|---|
| Automated Rust | 24 tests pass (23 core, 1 desktop): odd/portrait/tiny sizes, device/budget rejection, Retina/margins, transforms/inverse/rotation/zoom/black sampling, exact default seam, initial placement, history, migration/round trips, font fallback/collection face/cache reuse, texture generations/readers and existing editor tests. |
| Required checks | Formatting, locked workspace check/test, all-target Clippy with denied warnings, release workspace build and nine Python analysis tests pass. Existing `block 0.1.6` future-compatibility notice remains. |
| Programmatic native replay | Imports PNG alpha, edits/reorders text/images, dispatches local AppKit preview gestures, resizes to 1079 × 1919 and 2559 × 1439 with outputs open, undoes/redoes dimensions and a viewport gesture, pans/zooms/rotates, fits a viewport, changes name/color/font, saves/reloads, closes/reopens both mapped outputs, tests missing-font fallback, budget rejection, cancellation state and failed preparation retaining the live document. Pass. |
| Real input automation | Used actual mouse/keyboard in Settings → Canvas settings, typed 2559/1439, clicked Apply with outputs open, selected a text layer, opened Fonts, typed `Menlo`, selected Bold and used native Save As. Saved JSON confirms dimensions and `Menlo-Bold`; final build reopened that scratch project. The native menu and separate dialogs accepted input above the preview. |
| Static image inspection | Empty black canvas shows all four neutral edges. Font search and dimension fields are captured below. Captured mapped clean windows show composition without identification chrome, but stale captures prevent treating every screenshot as a current scene assertion. |
| Human / external displays | Not accepted. Unobscured physical picture continuity, repeated manual mapping/dialog cycles and physical seam/output verification remain pending. No external displays or physical scanout were verified. |

[Empty boundary](evidence/canvas-mapping-boundary.png), [Settings after keyboard entry](evidence/canvas-mapping-settings.png), and [font search with keyboard focus](evidence/canvas-mapping-font-picker.png) are static owned-window captures, not motion evidence. During the real-input check an off-canvas outline extended over the inspector; explicit preview clipping was added and the final native build/replay passed afterward. Inspect that edge behavior during human review too.

### Native transition trials and limits

```sh
# Rebuild the PID-scoped AX helper if needed:
clang -fobjc-arc tools/native_editor_window.m -framework Foundation -framework ApplicationServices -o /tmp/rastera-editor-window
clang -fobjc-arc tools/native_window_info.m -framework Foundation -framework CoreGraphics -o /tmp/rastera-window-info
python3 tools/canvas_editor_probe.py --mapping --out /tmp/rastera-mapping-final-captured --cycles 3
python3 tools/canvas_editor_probe.py --mapping --out /tmp/rastera-mapping-overlap-final --cycles 3 --no-capture
python3 tools/canvas_editor_probe.py --mapping --foreground --clean-only --out /tmp/rastera-mapping-foreground --cycles 3
python3 tools/summarize_canvas_editor.py /tmp/rastera-mapping-final-captured --images
python3 tools/summarize_canvas_editor.py /tmp/rastera-mapping-overlap-final
python3 tools/summarize_canvas_editor.py /tmp/rastera-mapping-foreground --images
```

Accessibility is required for the launching app's native AX driver; Screen Recording is needed for captures. The replay is opt-in and writes scratch projects/fixtures. It dispatches local preview events without global input; AX presses actual native minimize/close buttons, and restore clears the native minimized state. The dimension/font UI test above separately used real global mouse/keyboard input with user permission.

All four recorded trial runs confirmed three minimize/restore cycles separately for operator, Left and Right (18 state changes each), recovered and shut down normally. [Result data](canvas-mapping-results.json) retains zero/absent presentation evidence and frame IDs; logs/captures remain in the listed `/tmp` folders. The captured run preceded final preview clipping and selected-footprint overlap priority; the uncaptured replay was repeated on the final build with an intentionally overlapping footprint. An additional final-build foreground trial explicitly activated the test app, captured clean windows only, and restored previous app focus afterward.

| Trial | Largest logged operator UI gap by case: operator / Left / Right | Clean output presentation evidence |
|---|---|---|
| Captured | 593.004 / 573.806 / 585.657 ms | Operator/Left cases have no positive presentation reports. Left during the Right case reports a 5654.186 ms gap and a 5587.549 ms scheduling gap. This is a remaining stall, not a continuity pass. |
| Earlier background, no captures | 845.251 / 555.999 / 557.251 ms | Operator/Left intervals contain no usable scheduling/presentation records for the other outputs. Left during Right reports 297 positive presentations, maximum gap 33.334 ms. The missing intervals prevent general acceptance. |
| Final replay, no captures | 579.065 / 567.574 / 561.597 ms | Both outputs report 323 positive presentations in the operator case; other-output gaps in all three cases are at most 25.001 ms. |
| Foreground, clean captures only | 592.195 / 569.914 / 567.291 ms | Both outputs report 691 positive presentations in the operator case, max gap 37.501 ms; other-output gaps are 41.667 ms (Right during Left) and 33.334 ms (Left during Right). |

Whole-run zero timestamps (preview/Left/Right) are **784/870/552** for the earlier captured trial, **51/41/39** for the earlier uncaptured trial, **63/32/32** for the final uncaptured replay, and **93/65/69** for foreground clean captures. Corresponding missing presentation-handler counts are **0/1/2**, **0/1/0**, **0/3/4**, and **0/6/3**; missing GPU completions are zero. The result file retains the affected frame IDs. Native in-flight maximum is 2 in the captured run and 1 uncaptured, within the existing bound. The active four-slot pool logged at most one leased slot; uncaptured producer skips reached 1. Counts are sampled and do not measure total retired-generation memory.

The first three were background/PID-scoped trials on a machine in concurrent use. The foreground clean-only run is a separate visibility control; it does not erase the unfavorable earlier capture result. An onscreen flag does not establish unobscured visibility. Output scheduling was absent in some intervals; occlusion was not controlled, so those intervals cannot prove transition-only continuity or a GPU deadlock. Capturing the composited operator can stall native callbacks; snapshots can combine stale egui/Metal content with newer overlay content. Identical snapshots are not by themselves a reliable source-frame trace. Callback times are never substituted for zero/missing `presentedTime`; API presentation is not physical scanout. The operator transition pause and capture limitations are unresolved.

In the foreground clean-only run, the Right output marker was detected in every idle/operator/Left-case capture and changed centroid between adjacent captures. Left's rotated/cropped marker was missing in 13 of 24 operator-case and 14 of 24 Right-case captures; those samples are retained and do not establish continuity. Image analysis now tracks both X and Y, so a rotated marker moving vertically is not misreported as frozen because its X stays constant. Request spacing reached approximately 735 ms, and no pair of changing captures was fully bounded inside one operator callback gap. These are moving-picture observations across the transition trials, **not proof that no shorter transition pause occurred**. The foreground API gaps are stronger scheduling evidence, but human unobscured-picture acceptance remains pending.

A [mapped clean output with Menlo Bold](evidence/canvas-mapping-clean-output.png) shows black out-of-bounds sampling and composition without operator borders/labels/handles. It is a static capture.

A separate two-second release headless-generator smoke run and the final empty-boundary launch succeeded. This is not sustained-performance testing.

## Human review still needed

With windows unobscured, verify resize/undo/redo with outputs open; pan/zoom/rotate and numeric controls; Fit Canvas/Reset; partially off-canvas objects and inspector clipping; font search/styles/default fallback; dialog/menu focus; project reopen and output close/reopen. Confirm no canvas border, labels, arrows or selection handles appear in clean outputs. Enable the diagnostic background for motion checks, then minimize/restore the operator and either output repeatedly while watching the other outputs. Test default seam geometry at 1:1 physical pixels on real outputs separately.

There is no display assignment/fullscreen routing, shared presentation clock/genlock, calibrated color, video/audio, capture, NDI/DeckLink, slides, project packaging or Windows implementation. Broader capture continuity and first-canvas acceptance remain pending.
