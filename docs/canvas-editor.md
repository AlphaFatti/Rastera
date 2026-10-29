# Canvas editor — first review implementation

This editor builds on shared-canvas integration commit `9fbf7f81c2528ccf96143391adc0646bd9f6e976`, on `feature/canvas-editor`. This is a WIP checkpoint for remote review; it does not signify milestone acceptance. First-canvas, human visual and production acceptance remain pending. The operator's AppKit transition pause and capture-conditioned native callback stalls described in [the integration record](shared-canvas-integration.md) remain separate limitations.

The editor has since gained configurable dimensions, viewport mapping, installed fonts and a brand theme on `feature/canvas-mapping`. See [the current controls, schema and verification record](canvas-mapping.md); timing/input observations below describe the preceding checkpoint.

## Run and edit

macOS 14+ on the current Apple silicon test host, Command Line Tools, and Rust 1.99.1:

```sh
cargo build --release --workspace --locked
cargo run --release --locked -p rastera-desktop --bin canvas-editor
# Optional initial project, or stage without clean windows:
cargo run --release --locked -p rastera-desktop --bin canvas-editor -- --project=/absolute/path/show.rastera.json
cargo run --release --locked -p rastera-desktop --bin canvas-editor -- --outputs=0
```

The title **Rastera — Canvas Editor — Operator / UI preview** identifies the editing window. **Rastera — Native clean output — Left/Right** remain clean picture windows. The toolbar explicitly labels **Live Edit — changes affect outputs**: edits go live as soon as a complete composition is ready. This is not a preview/program switcher.

- Import local PNG/JPEG through the native panel (`Cmd+I`). PNG alpha is preserved; initial placement centers the image and fits it inside the current canvas without upscaling. Add text creates an editable multiline text object.
- Select by bounding box in the native preview or layer list, drag to move, and drag one of four corner handles to resize. Locked layers cannot be moved/edited; hidden/locked layers are skipped by canvas hit testing. The list can select them for show/unlock. Images can lock to their original aspect ratio. Text box resizing changes wrapping, not font size.
- Inspector: name, canvas-pixel X/Y/width/height, opacity, image aspect lock; for text, multiline content, font pixel size, RGBA components and left/center/right alignment. Partially off-canvas objects are clipped by the canvas viewport. The operator-only outline may extend into preview letterboxing; clean outputs only sample the canvas.
- The layer list is topmost first. Up/Down reorder; duplicate gives a new stable layer ID (and a distinct editable text object), delete removes the layer and now-unused source objects. Show/hide and lock/unlock are per layer.
- Undo/redo: buttons or `Cmd+Z` / `Shift+Cmd+Z`. A completed native drag/resize produces one history entry, while complete scene updates can appear during that gesture. Inspector edits group by focus/drag sessions. History is limited to 100 entries, is not saved, and preserves the ID allocator's high-water mark across undo branches.
- `Cmd+D` duplicates and Delete/Backspace deletes only when a text/numeric field does not own keyboard input. Native preview clicks return keyboard focus to the host and surrender egui field focus. File shortcuts (`Cmd+S`, `Cmd+O`, `Cmd+I`) are global.
- Left/Right checkboxes close/reopen clean windows. The selectable diagnostic background retains the original animated pattern and seam columns. A plain composition is static unless edited; enable that background for continuity tests.

## Document and preparation

`rastera-core::scene` owns the versioned document, distinct `images` and `texts` object maps, ordered layers and stable IDs, geometry, hit testing, resize math, history, image inspection and project IO. No egui interaction state or glyph/GPU caches is serialized. `rastera-core::composition` owns text layout/raster preparation, asset caches and reusable GPU composition. Desktop interactions live in `rastera-desktop/src/editor.rs`; the original executables remain available.

PNG/JPEG decoding, media metadata, JSON IO and epaint 0.31.1 text layout run on file/preparation workers, not the producer. Canvas text uses the same pinned font/layout generation as the UI, with bundled defaults or a requested installed face (see the current guide). [Bundled font notices](../THIRD_PARTY_NOTICES.md) are included. Text is rasterized into cached RGBA tiles sized to its wrapping box. Cached GPU texels are premultiplied before upload, linearly filtered and composited in layer order with opacity; transparent edges do not blend arbitrary transparent RGB. This is an SDR prototype, not calibrated color management or a professional typography engine.

One request slot holds the latest complete document, one preparation operation runs, and one ready snapshot awaits a frame boundary. Intermediate requests coalesce. The producer consumes complete prepared scenes without waiting on decoding/layout/IO; the last ready composition keeps rendering on preparation errors or while loading. A newer pending edit does not discard an already completed snapshot and starve dragging. Resource keys include image path/mtime/size or text/style/box size; positions and opacity reuse prepared image/text textures. Prepared snapshots are immutable. Retired resources remain owned while the producer uses them; final cache/resource destruction normally occurs on the preparation worker. Queue uploads precede the producer's composition submission on the same wgpu queue.

The independent monotonic producer and native consumers preserve the existing ownership protocol: four 3840 × 1080 RGBA8 textures, publish only after producer GPU completion, native exact-device texture sharing, and frame leases held until native GPU completion before slot reuse. No CPU canvas/frame readback or eframe surface acquisition is added. Native consumer work and telemetry remain bounded. Each window can present a different latest completed frame; no genlock is claimed.

The AppKit preview child view now follows the central egui layout rectangle on main. Events carry actual drawable dimensions and backing scale; mapping uses the actual aspect-fit rectangle, including letterboxing. Window resizing changes the view, not document geometry. A separate transparent AppKit child overlay draws selection chrome above the preview; it is never encoded into a shared texture or attached to clean consumers.

## Projects and limits

Save/Save As write version-1 JSON with fixed canvas dimensions, source maps, ordered layers and ID allocator. Media references are made relative to the project directory; load resolves them against that directory and canonicalizes working references. Images are referenced, not packed/copied into the project. Save uses a sibling temporary file, sync and rename. JSON parsing, reference/dimension validation and complete GPU resource preparation must succeed before a loaded document replaces the current editor document. Errors retain the existing document and ready composition, and appear in the status area.

Current limits: 128 layers, 256 image/text source objects, 8192 px per image/box axis, a 32 Mi-pixel combined prepared-source budget, text font sizes 4–512 px, up to 4096 editable characters in the inspector, 8 MiB project JSON. Image imports are bounded before decoding. JPEG pixels are decoded as stored; EXIF orientation and embedded ICC transforms are not normalized. Glyph layout has the limitations of epaint's font/shaping implementation. The later canvas-mapping milestone adds installed-font and viewport controls; rich text, complex-script certification, embedded media, video/audio and slides remain unimplemented. Undo history is only in memory; no automatic recovery or unsaved-close prompt is implemented. Save before closing/opening another project.

## Automated verification

On Apple M1 Pro / macOS 15.5 / Rust 1.99.1, the final source passes:

```sh
cargo fmt --all -- --check
cargo check --workspace --locked
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo build --release --workspace --locked
python3 -m unittest discover -s tools -p 'test_*.py'
```

Fifteen Rust tests (14 core, one desktop) and nine Python analysis tests pass. Focused coverage includes half-open slice boundaries, Retina/letterboxing mapping, ordered/hidden/locked hit testing, corner anchors, handle grab offsets, very thin image aspect ratios, text box/style independence, undo/redo/ID high-water marks, PNG alpha/JPEG validation, relative project round trips/errors, wrapping/alignment and glyph clipping. The existing transitive `block 0.1.6` future-compatibility warning remains; current Clippy with denied warnings passes. Build output, temporary toolchains, machine editor settings and Python caches remain ignored.

Dependency additions: core directly uses existing `epaint 0.31.1`, `image 0.25.10` (PNG/JPEG enabled), `serde 1.0.229` with derive, and new `serde_json 1.0.154` (`itoa 1.0.18`, `zmij 1.0.23`). Desktop directly names existing `winit 0.30.13` only to keep explicit background probes from activating other applications. The native panel adds Apple's system `UniformTypeIdentifiers` framework. Existing eframe/wgpu/native-bridge versions remain unchanged; Cargo.lock is included. Text layout APIs were checked against [epaint 0.31.1 Fonts](https://docs.rs/epaint/0.31.1/epaint/text/struct.Fonts.html) and the installed implementation.

## Native editing and lifecycle verification

The final explicit local replay imports a real transparent PNG through the same file worker as the UI, creates and changes editable text, dispatches local AppKit NSEvents to the actual preview view, and checks resulting document geometry/history. It tests movement across the seam, image aspect-locked corner resize, text wrapping-box resize with font size unchanged, negative X/Y clipping positions, layer reordering, distinct duplicated text/deletion, save/reopen, missing-media load preserving the live document, and native output close/reopen. It runs without activating the app, posting system input or moving the cursor. These are programmatic assertions, not human input acceptance.

The replay exposed an actual lifecycle edge: a native output closed during a UI pass/modal file panel could be recreated from the pre-panel checkbox snapshot. The editor now retires closed handles before controls and lets a native close during the pass win over that stale snapshot. A subsequent checkbox request creates a fresh native consumer. Both outputs close/reopen in the passing replay. Complete prepared snapshots are also consumed even when another edit is pending, avoiding starvation during continuous dragging. A final zero-motion arithmetic guard prevents round-off-only resize/history changes.

During the user's separately granted mouse interval, direct UI automation exercised the **real PNG picker**, Add Text button, multiline input, text rename and center alignment, numeric X entry, native dragging and corner resizing of both image and text across the seam, layer Up, document undo/redo, the **real Save/Open panels**, native output close buttons and actual Left/Right checkboxes. The saved JSON has exactly two layers after Backspace and Cmd+D were sent inside the text field; text remained `Live Rastera text\nCrossing the output seam`, font size 80 and center alignment. Image geometry was approximately `(1750,240,1657.5,828.75)`; text `(1676.92,345,1150.07,703.75)`. The operator's actual yellow minimize control was clicked, native state read back minimized, and restoration recovered. This session used the preceding review build; the subsequent zero-delta guard and probe-only animation/title corrections passed the final checks and replay. [Operator editing capture](evidence/canvas-editor-operator.png).

The directly edited composition was saved and reopened at `/tmp/rastera-editor-direct.rastera.json`; its process was preserved for human review. The earlier observed `Picture1.png` placement was separately reconstructed at `/tmp/rastera-editor-recovered.rastera.json` after a same-name System Events selection ambiguity closed that initial window. New automation resolves AX windows directly by PID and labels its windows `(automated probe)`. No user image bytes or private media paths are included in this repository. The self-contained [editable demo](../examples/README.md) uses only the synthetic fixture and direct-test text/geometry.

## Native transition observations

The final animated trials use one matched release executable, SHA-256 `03e9400f7cfb3395e9f5500d7329eae56d4378d07b5173bee798cf3f0746c551`, default macOS animations, Reduce Motion off and unchanged App Nap behavior. The Mac was also in use for other work; no throughput certification follows. Each trial includes idle activity and **three minimize/restore cycles separately for operator, Left and Right**, using PID-scoped native minimize buttons and `AXMinimized=false` restoration. Captures are only owned window images, never unrelated desktop regions. Each native state/action was confirmed. Automated probe startup intentionally avoids activation; normal editor startup behavior is preserved.

| Minimized window | Still-visible clean output(s) | No-capture maximum valid Metal presentation gap | Captured maximum valid gap | Captured content |
| --- | --- | ---: | ---: | --- |
| Operator | Left / Right | 16.674 / 16.674 ms | 37.500 / 37.500 ms | 24 samples each; marker detected and changed at every successive sample |
| Left | Right | 33.333 ms | 33.334 ms | 24 samples; all detected/changed |
| Right | Left | 16.682 ms | 41.667 ms | 24 samples; all detected/changed |

One pair of Left images was acquired wholly inside the **same 584.580 ms operator UI callback gap**; its marker centroid changed `282.5 → 40.3425` thumbnail pixels (wrapping across the edge). See [first](evidence/canvas-editor-gap-first.png) / [last](evidence/canvas-editor-gap-last.png). This is image-content evidence in addition to presentation records. The operator UI still pauses: maximum recorded gaps were 586.380 ms in the uncaptured operator trial and 593.502 ms in the captured trial. This implementation does not fix that controls pause.

Sampling is sparse: maximum capture request spacing was 753 ms for operator/Left and 744 ms for operator/Right, 515 ms for Left/Right and 534 ms for Right/Left. Acquisitions lie between logged start/return times. Changed sampled images do not exclude shorter freezes, missed refreshes, occlusion, compositor caching or physical scanout problems. No camera, photodiode, projector or human continuity acceptance was obtained. The earlier composited-operator capture limitation remains unresolved and is not overturned by this run.

All completed trials drained native GPU work and ended with zero outstanding canvas slots. Logged leased slots stayed at most 1 (uncaptured) / 2 (captured) of 4, with no logged pool-full skips. Observed native GPU commands in flight were 1, below the enforced bound of 2. There were no logged GPU/consumer errors, lost telemetry or missing GPU-completion records. **Presentation is separate:** whole-run zero timestamp counts for preview/Left/Right were `23/28/34` without capture and `64/73/78` with capture; missing presentation-handler counts were `0/3/0` and `0/3/1`. IDs and frames without positive presentation reports remain in [the result data](canvas-editor-results.json). Zero/absent presentation values are never replaced by CPU callback times.

Earlier background trials accidentally used a static composition (diagnostic background off). Those still test edit/lifecycle assertions, but their missing marker detections and absent/zero presentation data are **not motion evidence**. Their summaries are retained in the result data, and full logs/captures remain under `/tmp/rastera-editor-final/{uncaptured,captured}`. The corrected `animated-*` trials above are the continuity observations.

### Geometry and clean content

At 2x backing scale, native output frames were resized to 960 × 568 points, producing 1920 × 1136 captures including 56 pixels of title chrome and an exact 1920 × 1080 content area. At source row 200, Left columns 1917/1918 were blue and **only 1919 green**; Right column 0 was magenta, column 1 grid and column 2 orange. [Left seam](evidence/canvas-editor-left-seam.png) / [Right seam](evidence/canvas-editor-right-seam.png). This checks host integer mapping while images/text are composed, not two physical projection edges. The final replay also saves/reloads the negative-position image and unchanged 3840 × 1080 document after resizing presentation windows.

[Left](evidence/canvas-editor-left.png) / [Right](evidence/canvas-editor-right.png) contain composition and test pattern, with no editor chrome. The operator capture has a visible cyan selection outline; a profile-tolerant cyan-pixel check finds no such pixels in either clean capture. More fundamentally, selection is absent from the core document/render pipeline and exists only in the preview child overlay. Human checks should still inspect clean outputs while selecting, locking and dragging.

## Reproduce and remaining human checks

```sh
xcrun clang -fobjc-arc -framework Foundation -framework CoreGraphics \
  tools/native_window_info.m -o /tmp/rastera-window-info
xcrun clang -fobjc-arc -framework Foundation -framework ApplicationServices \
  tools/native_editor_window.m -o /tmp/rastera-editor-window
python3 tools/canvas_editor_probe.py --out=/tmp/rastera-editor/uncaptured --cycles=3 --no-capture
python3 tools/canvas_editor_probe.py --out=/tmp/rastera-editor/captured --cycles=3
python3 tools/summarize_canvas_editor.py /tmp/rastera-editor/captured --images
```

Accessibility and Screen Recording must be available to the launching app for the native window/capture driver. The replay itself uses no global input. `--editor-probe=FOLDER` is explicitly diagnostic: use the fixture generator/driver above in a scratch folder; it writes test projects and deliberately attempts a missing-media load. Normal editor runs do not execute it.

For human review, run the editable example command above or use the open direct-test window. Import your own transparent PNG and JPEG; type/change text, its color, font size and alignment; drag/resize across the seam and partly outside the canvas. Check numeric geometry, show/hide/lock, duplicate/delete, reorder and whole-gesture undo/redo with text-field focus. Save before opening another project or quitting. Resize the operator/outputs, close/reopen outputs, and watch the unobscured clean pictures during operator and output minimize/restore. Check handles never appear in them. Repeat with actual output displays. **Human visual and overall first-canvas acceptance remain pending.**
