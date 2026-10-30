# Media and scenes development

Branch `feature/media-scenes`, based on capture checkpoint `c813e6a`. Version remains 0.0.2-dev; v0.0.1 is unchanged. Work is delivered sequentially; later sections record only implemented features and actual checks.

## Stage 1: source cropping

Schema 6 explicitly migrates versions 1–5, preserving IDs, geometry and full-source crop defaults. Image and camera layer instances store normalized left/top/right/bottom bounds. Duplicated cameras keep one definition/session and independent crops. The layer rectangle is the visible cropped rectangle; UVs select its source region. Cropping changes that rectangle while preserving underlying pixel scale and full-source position, rather than stretching surviving pixels. Reset restores the uncropped geometry; resize operates on the visible aspect. Normalized bounds retain their meaning when a camera mode changes.

Select an image/camera, edit **Source crop · pixels**, or enable **Edit crop with corner handles**. Turn that mode off to resize. Dragging the body still moves the object. Text does not offer source cropping. Crop is undoable; a completed handle gesture produces one history entry. Viewports mode and Option-click behavior are unchanged. Controls report requested camera dimensions, which equal validated delivered dimensions for a Live camera; unavailable cameras use their persisted mode.

Automated workspace tests include crop/reset pixel-scale geometry, empty/NaN rejection, crossing-handle constraints, independent shared-camera crops and undo. Native picture/handle verification for this stage is pending. Existing operator-transition, screen-capture and physical-display limitations remain. No production acceptance is claimed.

## Stage 2: file playback

Schema 7 adds reusable local assets and separate playback IDs. Re-importing creates an independent playhead; duplicating a layer deliberately shares its playback. Video/GIF layers use the same geometry, crop, opacity and history as other picture layers. Play/Pause, Restart, Seek, file-default/override looping and Hold Last Frame/Black are available. Audio is explicitly muted. Missing/unsupported files report errors without device substitution.

AVFoundation AVPlayerItemVideoOutput delivers BGRA on each playback's dedicated worker, with preferred orientation applied via native video composition. Current native capability probing rejects missing/unplayable video and HDR transfer tags. image-rs 0.25.10 (unchanged) now enables its GIF feature, adding gif 0.14.2 and color_quant 1.1.0 to Cargo.lock. Its streaming compositor handles partial frames, transparency and disposal. Zero GIF delay uses a documented 10 ms minimum. GIF decoding retains only current/next frames and decoder composition state; seeking streams from the beginning. File loop metadata applies unless overridden. This is bounded work, not a promise of instant long-clip seeks.

Limits: four playback instances, sixteen referenced file assets, SDR up to 1920×1080 pixels (either orientation), duration ≤24 hours; GIF files ≤64 MiB / 10000 frames. CPU BGRA staging and GPU uploads are used, not zero-copy playback. One latest slot per playback replaces old frames. File slots deliberately have no capture TTL: paused/end-held/long-delay pictures remain valid. Input uploads and the unchanged shared-canvas completion ordering protect texture reuse. Workers, decoding and clocks do not depend on egui repaints; static resources are not prepared per media frame. Loop/end-setting edits currently restart their playback worker. Tests cover timing, transparency/loop fixtures, paused clocks, held frames and crop/model regressions. Native picture verification is recorded below when complete.

API references: [Apple AVPlayerItemVideoOutput pixel ownership](https://developer.apple.com/documentation/avfoundation/avplayeritemvideooutput/copypixelbuffer(foritemtime:itemtimefordisplay:)), [pinned image-rs GIF decoder/compositor](https://docs.rs/image/0.25.10/image/codecs/gif/struct.GifDecoder.html). No seamless/sustained playback, calibrated color or physical synchronization is claimed.

Stage-2 verification: workspace tests, denied-warning Clippy and signed release bundle build pass. An owned native synthetic H.264 MP4 (320×180, 30 fps, 6 seconds) plus GIF (320×180, 5 fps) replay verified advancing decoded video, a >3-second paused picture without TTL blackening, restart/seek, project round trip and a windowed clean consumer. Shutdown reported capture drained=1 and 0/4 canvas leases. The initial round-trip assertion exposed last-bit f64 JSON parsing differences in summed GIF duration; enabling serde_json's float_roundtrip feature corrected it without upgrading versions. GIF loop extension handling distinguishes absent metadata (once), zero (infinite) and finite additional repeats. Human video/GIF picture, mixed capture composition and fullscreen checks remain pending at this stage. Scratch logs/fixtures: /tmp/rastera-media-smoke; no personal camera pictures are recorded.

## Stage 3: canvas backgrounds

Schema 8 adds scene-content backgrounds: solid RGBA, an image asset, or an existing video/GIF playback, with Fit / Fill / Stretch. Fill is the default and aspect-preserving; background playback is set to loop when assigned. Import media first, select it in the bottom background inspector, and remove its ordinary layer if only the backdrop is wanted. Reference pruning retains background-only sources. The background is composed beneath ordinary layers; the diagnostic pattern remains an explicit override. #17151A workspace panels/preview margins are unchanged. Geometry tests cover Fit letterboxing, centered Fill crop and Stretch. Native/human background picture checks remain pending until the final combined replay.

## Stage 4: quick scenes

Schema 9 moves ordered layers, background and the diagnostic flag into persistent named scenes. Versions 1–8 explicitly migrate to one initial scene, preserving source/project/viewport/layer IDs and framing. The project retains canvas size, reusable source definitions and output destinations. There are 1–16 scenes, each at most 128 layers, with a separate monotonic scene allocator retained across undo branches. Add/Duplicate/Rename/Delete/reorder controls and quick cut buttons appear above the editor. The dot denotes the last completed live scene; highlighting denotes the requested scene. Scene duplication creates independent placement/text IDs and deliberately shares camera/playback references.

Cuts submit asynchronous full snapshots, preserve outputs, and never reload a project. The producer keeps the old composition until the newest requested revision is ready; stale ready revisions are discarded at frame boundaries. References/pruning/validation scan inactive scenes and backgrounds. Camera sessions remain shared across scenes. Media defaults to continue/resume even while inactive; enable Restart when scene activates per playback when desired. Shared looping backgrounds can therefore continue across cuts. Restart/loop policy belongs to the shared playback ID, not an independent clock per reference. Required tests/Clippy pass, including legacy migration, inactive background retention, scene IDs/history and original viewport split. Native rapid-switch verification remains pending until final replay.

## Stage 5: last-session recovery

Restore Last Session defaults on; Restore enabled outputs on startup defaults **off**. Preferences and two bounded (16 MiB each) recovery journal files live in `~/Library/Application Support/Rastera/session`, separately from explicit project saves. A single latest snapshot is sampled about once per second and written on a worker after a 500 ms quiet/coalescing interval; ordinary close flushes the final document/playheads. A crash can lose the interval since the latest durable snapshot. Writes sync a temporary file then atomically rename; a previous validated snapshot survives corrupt-current fallback. Invalid current and previous snapshots start a usable empty session with a warning. Disabling restoration stops journal updates (preferences still persist); existing recovery files are retained.

Snapshots retain document identity, original absolute project path, unsaved edits versus last explicit save, active scene, mode/selection, operator position/size and playback position/play-pause state. Media references are already resolved to absolute paths; Save As retains ordinary relative project references. Missing files keep their definitions and show missing-path status; unavailable image/file pictures use black rather than rejecting the entire recovered document. Invalid project schema/changed image dimensions still reject transactionally. No recovery action requests camera authorization. Authorized exact saved camera IDs can resume; missing/unauthorized sources remain visible as statuses without substitution.

`--no-restore` skips reading recovery; `--project=...` always overrides it. `--session-dir=/absolute/scratch/path` isolates preferences/journals. All `--editor-probe=...` runs automatically use a session subdirectory under their probe folder and skip loading it, preventing probe edits from replacing the user's session. A fresh-process restoration test must omit --editor-probe and use the explicit isolated --session-dir instead.

Output restoration requires the preference, matching nonempty machine identity, a surviving viewport ID and unchanged exact preferred mode/display. Existing destination planning still rejects missing/ambiguous/conflicting targets; no camera/display substitution or permission prompt is introduced. Reopening a project through Open continues disabling outputs. Operator position/size, inspector panel widths and native windowed output frames are restored. Native clean-frame access/placement runs on main, is keyed by viewport ID, and relocates unreachable windowed frames to a reachable screen. Fullscreen still uses its exact display assignment; its retained windowed frame applies when returning to Windowed. Monitor-layout changes may require repositioning the operator; broader window-layout/hardware recovery remains a verification limitation.

Tests cover unsaved/saved identity separation, journal round trips, corruption fallback and unusable recovery. All required workspace tests and Clippy pass. Native fresh-process recovery verification is recorded in the final results below.

## Final verification and review

October 30, 2026, Apple M1 Pro / macOS 15.5. Required fmt/check/test/Clippy (`-D warnings`), locked workspace release build, signed Rastera.app build, plist validation and strict signature verification pass. **56 core tests + 8 destination-test executions + 1 desktop test = 65 Rust test executions**; nine existing Python analysis tests pass. Additional GIF coverage constructs portable partial-frame fixtures exercising Previous/Background disposal, transparency, delay and loop metadata. Scene creation validates the whole candidate transactionally before replacing the document.

```sh
cargo fmt --all -- --check
cargo check --workspace --locked
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo build --release --workspace --locked
python3 -m unittest discover -s tools -p 'test_*.py'
./tools/build-app.sh
# Requires ffmpeg only to generate synthetic local test files, plus native Accessibility access:
python3 tools/media_scene_probe.py --out /tmp/rastera-media-scenes-reviewed
# Normal review launch; enabled-output restoration remains off by default:
open target/Rastera.app
```

The owned, uncaptured replay combines a synthetic H.264/yuv420p MP4 (320×180, 30 fps, six seconds), a synthetic GIF (320×180, 5 fps, two seconds), FaceTime HD Camera (actual 1280×720 BGRA, negotiated 30 fps), transparent PNG and editable text. These tested formats do not certify every AVFoundation codec/container. Input-copy/output ownership is unchanged. Windows, audio playback/capture, NDI and native DeckLink SDK remain unimplemented.

| Check | Actual verification | Limit |
| --- | --- | --- |
| File frames / pause / seek | Video sequence advances, remains valid through >3-second pause, and resumes after seek/restart | Not sustained frame pacing or physical picture proof |
| GIF | Native latest-frame sequence advances; portable tests verify disposal/timing/alpha/repeats | Synthetic native demo is opaque; native transparent-GIF picture remains a human check |
| Source crop | Native view mouse events change UV crop; completed gesture has one history entry; Undo/Redo restore exact documents | Physical pointer/Retina ergonomics still benefit from human review |
| Mixed scene | Video/GIF/camera/image/text input updates upload through the one shared canvas | Input counters alone do not prove visible motion |
| Backgrounds / scenes | Solid, shared looping video and image Fit scenes save/reopen; rapid requests finish on newest scene; capture remains Live | No fade, frame-accurate cue or seamless playback claim |
| Output paths | Windowed output plus one OS-reported external fullscreen consumer remain enabled through scene cuts | Physical target/picture observation remains pending |
| Native transitions | Two actual operator minimize/restore button cycles complete with sources/consumers active afterward | No screenshots; this cannot exclude transition-only picture pauses |
| Recovery | Fresh process retains unsaved scene/content, project identity/path, playheads, editor layout; outputs disabled by default | Crash can lose the last sampling/debounce interval |
| Output recovery / overrides | Opt-in restores exact matching windowed/fullscreen assignments; no-restore opens empty; explicit project overrides recovery | Physical unplug/reconnect/ambiguous display not newly hardware-tested |
| Shutdown | Owned runs exit 0, capture drained=1 and 0/4 canvas slots outstanding | Finite trials, not endurance certification |

Restart-on-activation is issued by the producer when it adopts a different scene at a frame boundary, through a bounded atomic cue per playback; media workers consume it independently of UI callbacks. Continue/resume is the default, including inactive-scene playback. Decoder seek completion is asynchronous: first-visible-frame-at-zero synchronization is not guaranteed. Changing loop/end configuration currently recreates that playback worker and resets its clock. End Hold/Black and broader codecs still need deliberate human picture checks beyond the pause/seek replay.

Failures retained: the initial media round-trip test exposed f64 JSON parsing drift (fixed with float_roundtrip); native crop Undo exposed paint-time pixel-margin round-trip edits (fixed by applying only changed controls); an initial enabled-output recovery test copied journals without the probe's isolated destination store, correctly restoring only the exact matching windowed destination. --session-dir now isolates both stores, and the repeatable driver copies both. Earlier failed trials are not acceptance evidence.

No camera pixels/screenshots are committed. [Sanitized results](media-scenes-results.json) record driver timing as command bounds, not image freeze durations. Zero/unavailable presentation timestamps, uncaptured intervals, the existing operator/capture-conditioned pauses and historical all-window automation gap remain limitations. The user confirmed “Pictures and scene changes look correct” in response to the new windowed/fullscreen review composition. This is a scoped human report; deliberate end-state, native transparent-GIF, detailed crop/control and recovery ergonomics checks remain pending. Physical scanout, mixed hardware, denied permissions, disconnect/reconnect, color calibration and sustained operation remain unverified for this new feature set. Source-license selection and production/milestone acceptance remain pending.


A separate signed human-review copy is open with the saved exact Windowed/Fullscreen targets enabled via its isolated preference, under /tmp/rastera-media-scenes-reviewed/human-review. This does not change normal defaults or signify production/milestone acceptance. The native restart replay seeks to a later playhead, switches scenes and verifies reset after producer adoption; continue-mode background playback survives the return cut. Both restoration stores are copied by the repeatable driver, avoiding the earlier fixture mismatch.


**Human feedback:** “Pictures and scene changes look correct.” The user also reported disliking the UI and explicitly deferred a UI-overhaul plan to later. This checkpoint preserves the functional controls and records that design work as future scope. No new UI overhaul is included.
