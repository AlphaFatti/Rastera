# Capture devices — 0.0.2-dev review milestone

[Media/scenes work](media-scenes.md) extends this historical schema-5 checkpoint with schema 9, file playback, crops and recovery. The device/copy measurements below retain their original scope.

October 29, 2026. `feature/capture-devices` starts at the annotated `v0.0.1` development-preview checkpoint (`c3b2258`). Main remains unmerged. The user reports successful fullscreen operation for that checkpoint; specifically unverified display cases and earlier operator/capture-conditioned pause limits remain.

## Build and use Rastera.app

```sh
./tools/build-app.sh
open target/Rastera.app
# Open a saved project under the same application identity:
open target/Rastera.app --args --project=/absolute/path/show.rastera.json --outputs=0
```

The script builds the locked release editor, packages `target/Rastera.app`, and verifies its signature. Workspace packages are `0.0.2-dev`; bundle short/build version is `0.0.2`, with the full development version also stored in Info.plist. Bundle ID is **org.rastera.desktop.dev**. It includes NSCameraUsageDescription, the Continuity Camera discovery key and the camera entitlement, uses Hardened Runtime and ad-hoc development signing by default, and sets a stable identifier-only development designated requirement. Camera consent persisted across tested rebuilds. This local identity is not a notarized/distribution signature. Optionally set RASTERA_SIGN_IDENTITY to an installed signing identity; that path uses its normal certificate-based designated requirement and was not tested here. Atomic executable replacement preserves a running binary's inode.

Camera access is requested only through **Add Capture Device…**, source Retry/Reassign, or the explicit capture-test command. Choose Allow in macOS's Rastera camera prompt. The ordinary cargo-run editor remains available for non-camera work; unbundled camera actions explain how to launch Rastera.app rather than attempting capture under Terminal/VS Code. Denied/restricted access is reported; use System Settings → Privacy & Security → Camera → Rastera. No microphone entitlement, permission request, audio output or audio recording is added.

Choose an AVFoundation device and a supported resolution/frame-rate choice, then Add source to canvas. Built-in, external and Continuity devices are discovered only when exposed by AVFoundation. Only the listed, validated 8-bit modes within the resource limit are offered. OBS Virtual Camera is a software device, not evidence of a second physical camera/card. The native DeckLink SDK, audio, file-video decoding and NDI remain future work.

Capture layers use ordinary selection, drag/corner resize, numeric geometry, aspect lock, opacity, ordering, visibility, locking, duplication, deletion and history. Duplicate shares the capture definition/session; independently adding the same device twice reports a conflict rather than stopping the existing owner. Reassign device / format changes the shared definition. Unlocked aspect-locked instances fit the new source aspect within their previous bounds while preserving center; locked geometry stays unchanged. Retry reopens the exact requested source. Missing devices never substitute another camera.

The source inspector reports requested mode, actual BGRA dimensions, negotiated frame rate, mean observed callback cadence, delivered frames and GPU upload operations. Upload operations can exceed unique frames after document changes, because a newly adopted scene can upload the current frame again. “Live” means valid frames arrive; a virtual camera or capture card may deliver its own static placeholder/test card. Source status does not analyze optical motion or prove a connected video signal. Diagnostic labels remain outside clean output.

Starting, Live, No Signal, Disconnected, Permission Required/Denied and Error states are implemented. Silent stale input keeps its last frame for at most two seconds, then supplies opaque black pixels; a source still starting without frames reports No Signal after five seconds. Explicit disconnect/error/permission state clears the latest frame immediately. Ordinary layer opacity still applies to black pixels. Other sources, static layers and the independent canvas producer continue. Hidden instances still reference the source; deleting the final instance retires the session. Disable All Outputs stops projection windows, not camera sources. Remove the last source layer or quit to release cameras.

## Model, scheduling, copies and ownership

`rastera-core::capture` owns reusable Source/Mode definitions, validation, the latest-frame slot and source geometry/persistence. Document schema **5** adds a `captures` table and ordinary `Content::Capture(source_id)` references. Explicit v1–v4 migration preserves earlier geometry, IDs, allocator, image/text references and project identity (or the existing identity migration); migrated projects start with no capture definitions. Saved definitions retain exact device IDs, names and requested modes, not captured images. Missing/unauthorized devices do not reject otherwise valid projects. Opening a project still disables outputs, preserving the existing destination safety policy.

The practical limit is **two source definitions**, up to **1920×1080 pixels per source (either orientation, at most 2,073,600 pixels), 1–60 fps**, SDR BGRA with square pixels. HDR-tagged and non-square-pixel input is rejected with status rather than silently interpreted. This is not a calibrated color pipeline. AVFoundation mode negotiation can fail for a device-specific combination; the requested mode is retained and the operator can explicitly reassign it. Device unique IDs can change after driver/port changes; there is no name/index fallback.

`native/capture.mm` is a narrow AVFoundation adapter. Session creation/configuration/start/stop use a background serial session queue; each source has its own serial delivery queue. Retired pending starts are canceled. The adapter requests BGRA output dimensions explicitly and selects corresponding common macOS session presets where available, then validates actual delivered dimensions. Frame durations are clamped to the selected format's supported range. Rotation is explicitly 0° and mirroring is disabled. AVFoundation may convert/decode/scale device-native data before BGRA delivery; these internal copies are not measured.

Known copies:

1. AVFoundation owns the CMSampleBuffer/CVPixelBuffer through its delegate callback. The adapter locks BGRA read-only, validates type/transfer/aspect, and synchronously copies rows into a Rust-owned packed buffer, honoring source row stride and forcing opaque alpha. It unlocks before returning; no CV pointer escapes.
2. A single latest Arc<Frame> per source replaces superseded data. The producer uses try-lock snapshot acquisition and preserves the previous GPU input on brief contention. AVFoundation alwaysDiscardsLateVideoFrames also bounds callback backlog. There is no FIFO of captured frames.
3. At most one upload per unique source per canvas frame calls wgpu 24.0.5 Queue::write_texture, which copies into staging memory before returning. The producer's next submission orders that upload before composition on the same queue. Its existing wait for its own completed canvas submission prevents rewriting an input texture while an earlier canvas render reads it. Input GPU textures/black buffers are prepared/cached off the producer; data upload is a bounded producer operation. Native display consumers read only completed canvas textures and keep their existing completion-held leases.

This is **not zero-copy capture**. The canvas/display path still has **no CPU canvas readback**. No document submission, image reload or text rasterization occurs per captured frame. Static edits still prepare snapshots normally. Multiple layer instances use one input texture/session. Core slot keys include project identity and source ID; weak registry entries plus runtime/prepared owners avoid both cross-project aliasing and dead-source accumulation. Epoch fencing rejects callbacks from retired/reconfigured sessions. Stop removes observers/delegates and drains the delivery queue before releasing its Rust context. Shutdown waits up to five seconds for capture retirement and reports whether it drained. Rapid control edits can briefly queue canceled control operations; video-frame work stays bounded.

## Verification and actual observations

The v0.0.1 checkpoint passed the required checks and release build before its annotated tag was pushed. Current capture checks pass: required formatting, locked workspace check/test, Clippy with denied warnings, release workspace build, bundle signature verification, and nine existing Python analysis tests. There are **42 core tests**, four destination tests in each native desktop binary, and one desktop scheduling test (**51 successful Rust test executions**). New tests cover source sharing, final-reference pruning/history, schema-4 migration/schema-5 round trips with missing devices, source/mode limits, padded stride/channel order/alpha, latest replacement, epochs, stale/failed sources, nonblocking snapshots, retirement-held buffers and aspect-preserving reassignment. The existing `block 0.1.6` future-incompatibility notice remains; dependency versions were not upgraded.

```sh
cargo fmt --all -- --check
cargo check --workspace --locked
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo build --release --workspace --locked
python3 -m unittest discover -s tools -p 'test_*.py'
./tools/build-app.sh
# Actual camera action; may require user consent. Saves no camera pictures:
python3 tools/capture_device_probe.py --out /tmp/rastera-capture-final
python3 tools/capture_device_probe.py --out /tmp/rastera-capture-reopen --project=/tmp/rastera-capture-final/review.rastera.json
```

The PID-scoped native window helpers from [display outputs](display-outputs.md) are needed for the test driver's minimize/restore/close actions. Tests use local native replay, not global mouse/keyboard input. [Recorded metrics](capture-device-results.json) contain no camera pixels or hardware device IDs:

| Source / check | Actual result | Verification scope |
| --- | --- | --- |
| FaceTime HD Camera (model FaceTime HD Camera) | 1280×720 BGRA, negotiated 30 fps, observed about 30.10 callback fps in the recorded startup sample | Real built-in camera; finite mean, not sustained/calibrated timing |
| OBS Virtual Camera (OBS Camera Extension) | 1920×1080 BGRA, negotiated 60 fps, observed about 60.06 callback fps; input hash changed | Distinct virtual device; user supplied moving DVD screensaver |
| Composition | Camera, OBS inset, transparent PNG and editable text appear through the shared windowed/fullscreen path | Native uploads/consumers plus user picture observation |
| Sharing/lifecycle | Duplication kept one session; removing one kept it; final deletion retired it; undo restarted; save/reopen preserved source definitions | Programmatic native assertions |
| Format change | FaceTime changed to 640×480 @30, then back to 1280×720; OBS remained Live | Actual buffer-size/frame delivery assertions |
| Fresh-process reopen | Both saved sources became Live and uploaded in a new signed app process | Independent cold load, not only in-memory reuse |
| Minimize/restore | Two native operator cycles in each capture/cold-load trial; capture_delivery batches continued during minimized intervals | Callback activity, not physical presentation proof |
| Shutdown | Both final test processes exited 0, capture drained=1, canvas slots 0/4 | Finite resource-retirement check |

The first camera run exposed macOS overriding the requested 720p device format with 1080p output. Validation kept it black rather than uploading incompatible rows. Explicit output sizing/session negotiation corrected it. An early review then showed OBS's placeholder covering FaceTime because the second layer was on top; the user enabled the OBS DVD source, and the final review uses a separate inset. Initial “Live” OBS buffers alone were not treated as proof of a moving hardware picture.

**Human observation:** the user confirmed “Both pictures and overlays look correct” after seeing FaceTime as the large picture and OBS/DVD as a separate inset in windowed/fullscreen pictures. This supports that observed composition; it does not accept every format, permission/error path or transition. Camera pictures remain local; scratch projects under /tmp reference repository media and are not portable bundled shows.

**Pending:** a second physical camera/capture card, physical unplug/replug, hardware No Signal, Continuity Camera, denied/restricted permission through actual OS settings, broader format/device compatibility, color calibration and sustained performance. Existing operator pauses, zero/missing presentation evidence, capture-conditioned stalls and the historical all-window automation limitation remain; no general continuity or production acceptance is claimed. Audio, native DeckLink SDK and NDI remain subsequent work. Fullscreen/dynamic/editor native regression suites also pass on the capture branch.

API references: Apple [video data delegate queues](https://developer.apple.com/documentation/avfoundation/avcapturevideodataoutput/setsamplebufferdelegate(_:queue:)), [late-frame dropping](https://developer.apple.com/library/archive/technotes/tn2445/_index.html), [macOS camera authorization](https://developer.apple.com/documentation/bundleresources/requesting-authorization-for-media-capture-on-macos), [camera entitlement](https://developer.apple.com/documentation/bundleresources/entitlements/com.apple.security.device.camera), and installed macOS SDK declarations plus pinned wgpu 24.0.5 queue implementation. These are implementation references, not substitutes for pending hardware tests.
