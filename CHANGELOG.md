# Changelog

## 0.0.2 — early development preview — November 2, 2026

Implementation frozen at `7d2fb6106ead8310a37c428c29702edf186afb1f`; release metadata/packaging do not change the engine.

- Canvas editor: distinct image/editable-text objects, layering/history, configurable canvas and stable dynamic viewports with arbitrary logical dimensions and footprint scale.
- Shared offscreen Metal canvas, bounded texture generations and completion-held clean native consumers; assigned-display fullscreen and windowed outputs with explicit activation/recovery.
- PNG/JPEG, AVFoundation capture, native video/GIF, source crop, backgrounds, Quick Scenes, project/recovery migration and opt-in exact video-output recall.
- Four-corner perspective pinning, operator-only handles and transactional installed-app workflow.
- Ten stereo audio buses; discrete embedded/capture channels, Stereo/Dual Mono/Dual Stereo/Quad Mono, explicit track mappings, stable independent strips, DSP/sends, shared transport, native file decoding and Core Audio device patching. Schema 11 migrates older projects to silent audio defaults; master audio activation always starts off.
- Compressed read-only arm64 DMG, tester guide, final checksums, strict bundle/deployment/resource checks and source provenance. Current package is ad-hoc signed and unnotarized; no public GitHub Release.

**Still unresolved:** reproduced source seek/loop PCM gaps and short-loop continuity; operator/video/capture-conditioned pauses; physical A/V calibration, broader hardware and endurance beyond the recorded 600-second trial. The user reports click-free playback with their substituted VOG file; no fixture defect or general continuity acceptance follows. Prior native/human observations remain scoped in [audio verification](docs/audio-engine.md). This is not production-show or milestone acceptance.

Next: **audio stabilization → slides engine → UI overhaul → cue/queue engine**. The UI is being designed separately by the user.

## 0.0.1 — development preview — October 29, 2026

- Image/text canvas editor with layers, history, projects, custom canvas sizes and viewport mapping.
- Dynamic viewport identities, custom output dimensions and independent canvas footprint scaling.
- Shared offscreen Metal/wgpu canvas with bounded textures and completion-held native output leases.
- Assigned-display borderless fullscreen, display identification, conflict/missing-target handling and Disable All Outputs recovery.

The user reports successful fullscreen operation. This observation does not establish every transition/hardware case. Physical disconnect/reconnect, negative-origin arrangements, multiple external displays, broader Spaces/Stage Manager and sustained operation remain unverified. Existing operator transition pauses, zero/missing presentation evidence and capture limitations remain documented. This is a development preview, not production-show acceptance. Capture devices, audio, DeckLink, NDI and slides are not part of this checkpoint.
