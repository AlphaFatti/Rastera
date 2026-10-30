# Changelog

## 0.0.2-dev — capture-device review work

- Added per-layer source crops, native muted video/streaming GIF playback, canvas backgrounds, quick scenes and bounded startup recovery with opt-in exact output restoration. Schema 9 migrates existing projects. See docs/media-scenes.md for actual automated/native results and pending human/hardware checks.

- AVFoundation video sources as shared ordinary canvas layers, with bounded BGRA uploads, mode/device selection, source status, lifecycle sharing and schema-5 projects.
- Signed Rastera.app development bundle with stable camera identity/usage description/entitlement and explicit consent action.
- Real FaceTime HD Camera and OBS Virtual Camera tested together; user confirmed moving pictures/overlays in windowed and fullscreen outputs. Physical unplug/replug and a second physical device remain pending. No audio, DeckLink SDK or zero-copy capture claim.

## 0.0.1 — development preview — October 29, 2026

- Image/text canvas editor with layers, history, projects, custom canvas sizes and viewport mapping.
- Dynamic viewport identities, custom output dimensions and independent canvas footprint scaling.
- Shared offscreen Metal/wgpu canvas with bounded textures and completion-held native output leases.
- Assigned-display borderless fullscreen, display identification, conflict/missing-target handling and Disable All Outputs recovery.

The user reports successful fullscreen operation. This observation does not establish every transition/hardware case. Physical disconnect/reconnect, negative-origin arrangements, multiple external displays, broader Spaces/Stage Manager and sustained operation remain unverified. Existing operator transition pauses, zero/missing presentation evidence and capture limitations remain documented. This is a development preview, not production-show acceptance. Capture devices, audio, DeckLink, NDI and slides are not part of this checkpoint.
