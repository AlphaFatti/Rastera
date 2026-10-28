# FOSS Live Production App — Build Blueprint

Prepared for Tim Del Vecchio / TD Productions · October 28, 2026

**Status:** proposed product and engineering plan. This is a starting specification, not an implemented application or a claim of tested hardware compatibility. The first target is native Apple silicon macOS; Windows is a planned port.

## 1. Product direction

Build a live media server that combines an OBS-like scene editor, Resolume-like canvas and output mapping, and a cue-based slide workflow inspired by ProPresenter. It should serve small crews doing church services, corporate presentations, livestreams, and projection jobs.

The central object is a **logical canvas**. Its dimensions belong to the show, independently of the operator display, audience displays, capture devices, and SDI video modes. Sources and slides compose into that canvas. Output routes select regions or layers and map them to destinations.

One example show:

- A 3840 × 1080 audience canvas spans two 1920 × 1080 projectors.
- A separate 1920 × 1080 stream composition uses a camera and the same slide content in a lower-third template.
- The stream composition goes to NDI and, when supported by the connected device, a DeckLink output.
- A stage composition shows current slide, next slide, a clock, and speaker notes.
- Each composition can also open in a clean windowed projector for setup.

Physical outputs do not automatically display every layer. Stage notes must be excluded from audience and stream routes by their composition definitions.

## 2. Requirements translated into implementation work

| Requested capability | Proposed implementation | Important boundary |
| --- | --- | --- |
| M-series Mac support | Native arm64 application; GPU composition through Metal via wgpu | Establish minimum macOS and supported hardware through the first compatibility tests |
| Possible Windows version | Shared Rust document, routing, timing, and rendering code; platform-specific device adapters | Windows capture, GPU sharing, installers, and drivers still need their own implementation and tests |
| OBS-style canvas editing | Source list; drag, resize, rotate, crop, reorder, lock, hide; numeric transforms; snapping; undo | Editor overlays never appear in clean output textures |
| Custom canvas size | Integer pixel dimensions and rational frame rate stored per composition | Validate against GPU limits and estimated memory; never advertise unlimited resolution |
| Canvas split across outputs | Named slices with source rectangles, destination rectangles, scaling policy, rotation, and later warping | A destination may contain several slices, and a canvas region may feed several destinations |
| Mac display output | Borderless clean output window on a chosen display, with explicit arm/disarm | Use physical drawable pixels, not only Retina logical points |
| Windowed projection | Resizable clean output window using the same routed texture as physical output | Preview refresh can differ from show render timing |
| UVC and capture cards | AVFoundation device discovery and capture on macOS | UVC, vendor-driver capture, and DeckLink are distinct adapter paths |
| DeckLink input/output | Native C++ bridge to the Desktop Video SDK | Enumerate supported modes, channel profiles, duplex capabilities, pixel formats, and device availability |
| NDI input/output | Optional SDK-backed discovery, receive, and send adapter | Runtime license and attribution requirements remain separate from the application source license |
| Slide engine | Presentations made of editable text, image, shape, and media objects; templates and cue actions | File import compatibility is a separate feature from having a slide engine |

## 3. Architecture recommendation

### Preferred long-term direction

Use **Rust for the shared application core, wgpu for composition, and a native desktop shell**. An egui/eframe shell is a reasonable first candidate for an engineering-heavy operator UI. It supports native windows and permits fast iteration. Test text editing, accessibility, keyboard focus, docking, and multi-monitor behavior before committing to it for the mature product.

wgpu targets native Metal and DirectX 12 among other APIs. That makes it a credible portability foundation, but native video texture interoperability is additional engineering work. Do not assume selecting wgpu provides zero-copy capture, DeckLink transfers, or Syphon integration automatically. [1][2]

Use small Objective-C++/C++ bridges where the platform APIs or vendor SDKs require them. Keep those interfaces narrow: discovery, capabilities, start/stop, frame delivery, clock information, and output completion.

Use proven demuxing, decoding, audio, text shaping, and image libraries. On Mac, investigate AVFoundation/VideoToolbox and Core Video texture interoperability first; an audited FFmpeg build can broaden format coverage. Pin and record the actual dependencies once the initial spikes pass. Building custom video codecs is outside the first product scope.

### Evaluate libobs before paying for a new engine

Run a small **libobs feasibility spike** alongside the initial renderer investigation as sequential experiments, not two production implementations. Its media pipeline, scene/source model, audio subsystem, and module system can save substantial work. [3]

The deciding experiment is whether it can cleanly support this project's independently sized compositions, multiple mapped outputs, required scheduling, and clean slide sources. Compare the resulting design and measured performance with the dedicated renderer. If it satisfies those needs, building on libobs may be the most practical route. Reusing its code also determines GPL obligations; do not label that combined application Apache-only. [4]

**Provisional preference:** a dedicated canvas/output core if the libobs experiment requires pervasive pipeline changes. This recommendation is an engineering judgment, not a benchmark result.

### Runtime boundaries

1. **Show document and commands:** assets, source definitions, scenes, slide presentations, routes, cues, and undoable edits.
2. **Media services:** asynchronous asset probing, decode, preloading, capture, network receive, and device discovery.
3. **Live renderer:** consumes a validated snapshot and renders shared source frames into composition textures.
4. **Output mapper:** draws selected composition regions into destination textures.
5. **Destination adapters:** display/window presentation, NDI sending, and scheduled DeckLink playback.
6. **Operator UI:** edits documents, previews changes, dispatches cues, and reports health.

Keep file access, device discovery, font loading, asset probing, and blocking SDK calls off the render path. Use bounded queues. A slow receiver or minimized preview must not stall every other output.

Publish a new live scene snapshot at a frame boundary when a cue is taken. Maintain separate edit/preview and live state. Make automatic live editing an explicit mode if added later.

## 4. Canvas and routing model

Store these as separate concepts:

| Concept | Meaning |
| --- | --- |
| Asset | Referenced file or generated content definition |
| Source | A media, capture, slide, or generator producer with a stable ID |
| Source instance | Placement and playback behavior of a source inside a scene |
| Scene/composition | Ordered layers rendered at specified dimensions and frame rate |
| Bus | A named live role such as Audience, Stream, or Stage, referencing a composition |
| Logical output | A destination-sized texture assembled from one or more slices |
| Slice | A source selection and mapping into a logical output |
| Destination | A display, clean window, NDI sender, or DeckLink channel consuming a logical output |
| Cue | An explicit action or group of actions applied at a defined time |

For the two-projector example, use this exact pixel geometry. Rectangles use a top-left origin and half-open bounds, so the right edge of the left slice does not duplicate the left edge of the right slice.

| Route | Source canvas region `(x, y, width, height)` | Logical output | Destination |
| --- | --- | --- | --- |
| Audience left | `(0, 0, 1920, 1080)` | 1920 × 1080 | Projector A |
| Audience right | `(1920, 0, 1920, 1080)` | 1920 × 1080 | Projector B |
| Stream | Full independent 1920 × 1080 composition | 1920 × 1080 | NDI sender and/or supported DeckLink channel |
| Stage | Full independent stage composition | Configured stage resolution | Stage display or clean window |

Support fit, fill, and stretch explicitly. Store source selection and destination placement independently. Reserve mesh warp, edge blending, polygon masks, and per-output color calibration for later milestones.

Decode a shared media source once where playback state is shared, then reuse its frame across compositions. Permit explicit independent playback instances when two cues need different time positions in the same file.

Persist device preferences using stable identifiers and capabilities, with a manual reassignment path. Output preparation should work without connected hardware. Missing destinations remain visible as offline logical outputs.

## 5. Timing, audio, and pixel handling

These belong in the initial design even when the first interface is simple.

- Represent frame rates as rational numbers: `60000/1001` is distinct from `60/1`. Use integer/rational timestamps for media and scheduled output.
- Use one show timeline with adapters translating to device clocks. Select an appropriate master clock for the active configuration and measure drift. Do not make the UI repaint loop the show clock.
- Give each destination a bounded queue and explicit repeat/drop policy. Record late frames and queue depth. Cross-output physical synchronization requires suitable hardware/reference support; a shared software timeline alone does not guarantee genlock.
- Start with a documented SDR color pipeline and defined transfer functions, matrix coefficients, range, and alpha convention. Convert correctly at source and destination boundaries. Linear-light compositing and premultiplied alpha are sensible targets.
- Plan 48 kHz floating-point audio internally, with resampling, meters, mute, delay, and bus routing. Embedded audio for NDI/DeckLink and local playback must derive from the same timeline. Prevent unintended duplicate monitoring paths.
- Preallocate reusable frame buffers. Avoid full-frame CPU readbacks in the display path; measure and minimize copies at SDK boundaries. Treat native texture sharing as a performance spike, with a documented copy-based fallback if needed.

DeckLink output is scheduled video/audio playback through a device SDK, not an extra desktop monitor. An arbitrary canvas must be mapped into a mode the connected hardware supports. The SDK exposes mode checks and scheduled frame submission. [5]

Mac display count and available refresh rates depend on the specific Mac and connected displays. Software cannot promise arbitrary native display counts. Additional DeckLink or network destinations have their own hardware, bandwidth, and driver constraints. [6]

## 6. Slide engine scope

Treat a slide as an editable scene that produces a texture, including alpha when required. That lets the same slide content participate in projection, a camera composition, a lower-third treatment, or an SDI/NDI route.

### First useful slide workflow

- Presentation library and an ordered show playlist.
- Text boxes with wrapping, alignment, font selection, color, and safe-area guides.
- Images, simple shapes, background colors, and video backgrounds.
- Slide thumbnails, next/previous, direct selection, and keyboard cues.
- Cut and fade transitions; a controlled Take action.
- Independent background/media and slide text layers, so changing text can preserve a looping background.
- Clear text, clear media, clear all, freeze, and blackout as distinct controls.
- Basic templates and readable missing-font/missing-media reporting.

### Subsequent presentation features

- Audience and stream templates sharing slide text but placing it differently.
- Independent announcement loops and persistent logos/messages.
- Stage display with current/next slide, notes, clock, countdown, and stage-only messages.
- Cue actions, timed advances, audio/video triggers, and reusable macros.
- More complete typography, theme inheritance, timer bindings, and localization.

This layer-based approach matches useful presentation workflows documented by Renewed Vision, including different layer selections for audience destinations. It is a behavioral reference, not a requirement to reproduce their proprietary file formats or assets. [7]

Defer PowerPoint and ProPresenter project import. Exported image decks and plain text are a useful early interchange path. Native editable import requires a separately scoped compatibility project. Song databases, Bible translations, and content services also carry separate content and integration considerations.

## 7. FOSS and dependency strategy

The application must remain buildable and useful with optional vendor integrations disabled.

**NDI:** the official distribution guidance permits open-source projects to include the headers under MIT terms and dynamically load the NDI libraries. The binary runtime has separate SDK terms, including attribution/distribution requirements. An open-source adapter does not make the runtime open source. Verify the SDK version's included license before distributing a release. Dynamic loading is an integration mechanism, not a universal license exemption. [8][9]

**DeckLink:** use Blackmagic's documented SDK and installed Desktop Video runtime; verify supported Apple silicon device/driver combinations and the SDK redistribution terms. Open application code does not make vendor drivers open source. [5][10]

**Application license:** decide before importing substantial existing code. A permissive Apache-2.0/MIT route fits broad reuse in a new Rust core. A GPL route fits a libobs-based application and stronger sharing obligations. The choice should express the project's intended contribution model as well as dependency requirements.

**FFmpeg:** its effective license depends on build options. Record the exact configuration and preserve the corresponding source, notices, and redistribution obligations. Do not assume every downloaded binary is an interchangeable LGPL build. [11]

Use original branding, UI assets, and implementation. Compatibility goals can be described without copying competitors' source, graphics, or bundled content.

Consider Syphon on Mac and Spout on Windows as subsequent local-application adapters. Syphon has an openly licensed framework and Metal support, making it a useful interoperability candidate on Mac. [12]

## 8. Milestones and exit criteria

| Milestone | Deliverable | Exit criteria |
| --- | --- | --- |
| 0 — Technical proofs | Renderer/output prototype, libobs comparison, NDI/DeckLink feasibility notes | Real Mac test produces two clean outputs; measurements identify copies, frame pacing, SDK constraints, and the chosen engine |
| 1 — Canvas/output alpha | Custom canvas; images/color/text; layer transforms; save/load; clean window and display routes | A 3840 × 1080 grid splits exactly across two 1080 outputs; resizing operator UI does not alter output geometry |
| 2 — Media and capture | File playback, looping, preloading, UVC capture, basic audio buses | Two live sources plus a clip and overlay run without growing memory or unbounded queues; A/V offset measured |
| 3 — Presentation alpha | Editable slides, templates, playlist, cue controls, cut/fade, separate text/background | Operator can prepare and cue a short service or corporate run-of-show while live output stays stable |
| 4 — Network and SDI beta | NDI receive/send; DeckLink capture/playout; embedded audio; device health | Verified receive/send and supported-device tests at both integer and fractional frame rates; one offline destination does not stall others |
| 5 — Production candidate | Recovery, relinking, hot-plug policy, output arming, diagnostics, installer/signing | Repeated representative long-duration shows pass the published hardware test matrix |
| 6 — Windows and advanced mapping | Windows adapters and packaging; later warping/blending and show control | Same project geometry/cues pass on Windows; platform differences are documented and tested |

Investigate vendor I/O in milestone 0 even though its complete user workflow ships later. Do not postpone discovering an incompatible GPU/SDK design until after building the editor.

## 9. Verification that matters

Automate geometry, document migration, cue transitions, timestamp math, and error handling. Validate physical output behavior on actual hardware.

1. **Geometry:** use a grid with numbered columns and single-pixel boundary markers. Verify no duplicate/missing seam pixels in the 3840 × 1080 split. Repeat with Retina scaling and different output aspect ratios.
2. **Multiple outputs:** route the same composition to a window and a physical display, then add network/SDI destinations as available. Record frame timing per destination and CPU/GPU load.
3. **Mixed rates:** run a `60000/1001` composition against supported 59.94 video hardware and a 60 Hz desktop preview. Verify bounded conversion behavior and measure drift.
4. **Live editing:** drag sources, open inspectors, generate thumbnails, save, and change the next slide while the live scene plays. Confirm output continuity and separation of preview/live state.
5. **A/V:** use flash-and-beep material and record the destination. Measure sync, long-run drift, and manual delay behavior for each output type.
6. **Faults:** unplug UVC/HDMI, remove a network source, stop an NDI receiver, disconnect supported external video hardware, and open a show with missing files. Verify clear health state and stable unaffected destinations.
7. **Recovery:** interrupt a save and crash the UI in a test configuration. Confirm the last valid show remains readable and recovery behavior is documented.
8. **Endurance:** begin with a 4-hour representative load, then a 24-hour soak for workflows like an overnight charity stream. Watch resident memory, queue depth, late frames, reconnects, and temperature-related performance changes.

A reasonable initial performance objective is sustained 1080-class output on the selected baseline Mac under the defined test workload. Publish measured results with the exact Mac, macOS, devices, media, and route count before making broader 4K or output-count claims.

## 10. First implementation task for an AI coding agent

Use this deliberately small kickoff brief after selecting the runtime through milestone 0:

> Build the first native Apple silicon canvas/output prototype. Establish the project with a shared document model, rational frame-rate type, source rectangles, logical outputs, and destination interfaces. Render a procedural 3840 × 1080 numbered test grid into an offscreen GPU texture. Create two independent clean output windows, each selecting one 1920 × 1080 half. Add an operator view that edits slice rectangles numerically and previews the full canvas. Save/load a versioned JSON show. Include meaningful tests for slice boundaries, aspect-ratio mapping, and fractional timestamp math. Keep rendering independent from editor repaint and file operations. Report what was compiled and exercised on a real Mac. Do not add a simulated NDI/DeckLink success state; unavailable integrations must be reported as unavailable. Deliver build instructions, measured frame timing, and the next smallest verified change.

After that passes, add a single video source, then a capture source, then an editable text slide. Keep each addition reviewable and preserve a working output test throughout development.

## 11. Decisions to record before a public release

- Exact baseline Mac, minimum macOS, and at least one target Windows machine.
- Measured renderer/SDK interoperability and the final libobs-versus-dedicated-engine decision.
- Application license and optional-adapter distribution policy.
- Tested UVC device, Blackmagic model/connection, and supported output modes.
- Named show workload and published performance limits.
- Code signing/notarization and reproducible dependency/version records.

## Primary references

Sources reviewed October 28, 2026. Recommendations and milestone scope are proposed design choices, not guarantees supplied by these projects.

1. wgpu: native rendering backends — https://wgpu.rs/
2. egui: native windowing and UI framework — https://github.com/emilk/egui and https://github.com/emilk/egui/blob/main/crates/egui/src/viewport.rs
3. OBS backend design — https://docs.obsproject.com/backend-design
4. OBS license — https://github.com/obsproject/obs-studio/blob/master/COPYING
5. DeckLink output interface, mode checks, and scheduling — https://sdk-doc.blackmagicdesign.com/decklink-sdk/DeckLinkAPI/interfaces/idecklinkoutput.html
6. Apple display support — https://support.apple.com/en-ca/102555
7. ProPresenter output layers and per-screen Looks — https://support.renewedvision.com/hc/en-us/articles/13634000690323-ProPresenter-Output-Layers and https://support.renewedvision.com/hc/en-us/articles/360041407174-Using-Looks-to-Show-Different-Screen-Content-in-ProPresenter
8. NDI software distribution — https://docs.ndi.video/all/developing-with-ndi/sdk/software-distribution
9. NDI SDK licensing — https://docs.ndi.video/all/developing-with-ndi/sdk/licensing
10. Blackmagic SDKs and Apple silicon migration documentation — https://www.blackmagicdesign.com/developer/products/capture-and-playback/sdk-and-software
11. FFmpeg licensing — https://ffmpeg.org/legal.html
12. Syphon framework — https://github.com/Syphon/Syphon-Framework
13. ArtCraft's FilmCraft repository, architectural inspiration — https://github.com/storytold/filmcraft
14. Apple capture setup — https://developer.apple.com/documentation/avfoundation/capture-setup

NDI® is a registered trademark of Vizrt NDI AB. Resolume, ProPresenter, OBS, ArtCraft, Apple, and Blackmagic Design names are used as references; this proposed project is independent.
