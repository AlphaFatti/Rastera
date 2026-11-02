# Source corner pin and installed development app

November 2, 2026. Branch `feature/source-corner-pin` starts at d81ecad, preserving 0.0.2-dev and v0.0.1. The broader UI overhaul is deferred.

## Core geometry

Layer instances own a `Pin`, four normalized destination points ordered source TL, TR, BL, BR. Canvas coordinates have a top-left origin and positive Y down. The clockwise polygon order is TL, TR, BR, BL. Identity is (0,0), (1,0), (0,1), (1,1). `Crop::full_rect(layer.rect)` reconstructs the stable full-source plane. A double-precision homography maps full-source unit coordinates into normalized destination coordinates; the full rect maps these into canvas pixels. Crop bounds select a subdomain of this same mapping, so surviving source points stay fixed rather than stretching into the previous quad. Reset Pin changes only Pin, preserving crop, ordinary placement and source reference.

Rendering carries the homogeneous denominator into clip-space W and uses perspective UV interpolation. Both triangles therefore implement one projective transform, without an affine diagonal seam. Core geometry supplies forward/inverse transforms, visible corners, half-open polygon hit testing and anchored whole-shape resizing. Moving/resizing the ordinary rect transforms the full pinned plane; text resize still changes wrapping, while pin-only edits reuse the cached unchanged text tile. Normalized pin data never changes capture/playback definitions.

Finite, clockwise convex, invertible configurations are required. Crossed/concave/collapsed shapes, projective poles and poorly conditioned transforms are rejected. Conditioning thresholds: normalized cross products >1e-6 of squared span, positive homogeneous W ≥1e-4 with ≤10000 ratio, inverse norm product ≤1e8. Visible corners must lie within ±65536 canvas pixels; ordinary placement limits remain. Off-canvas valid geometry is clipped by the canvas render target.

Schema 10 explicitly migrates v1–v9 layers to identity pins, preserving project/layer/source/viewport IDs. Recovery journals use the same document migration, including their last-explicit-save document. Scene/layer duplication, history and recovery preserve independent pin instances. The four-texture producer, frame-boundary adoption, bounded generations, completion-held leases and GPU-only canvas display path are unchanged.

## Editor controls

Canvas mode has Transform / Crop / Corner Pin tools; Viewports selection and Option-click cycling are unchanged. Yellow TL/TR/BL/BR handles and numeric visible-corner canvas X/Y fields belong to Corner Pin. Source corner labels remain attached to source orientation even after rotating the shape. Reset Corner Pin restores rectangular mapping. The inspector marks active warps. Selection outlines and hit targets use the warped polygon; body dragging translates the whole source. Transform resizes the whole shape with the opposite visible corner anchored. Crop handles use the inverse full-source homography; text remains editable but has no source crop.

Handle hit radii derive from preview backing pixels/Retina scale and the actual fit rectangle. Each completed gesture has one history entry; selection alone has none. Escape cancels the gesture. Tool/mode changes finish it safely. Numeric/reset edits use normal history. Invalid edits keep the last valid geometry and show a warning. All handles/guides/labels remain exclusively in the operator overlay.

## Verification status

Pure geometry/migration/shared-source tests and native checks are reported separately below. No sustained performance, calibrated color, physical scanout synchronization or pause fix is claimed. Existing operator/capture-conditioned pauses and unverified hardware cases remain. Installation/signing/launcher results are recorded separately from picture observations.

## Build and installation

```sh
./tools/build-app.sh                 # optimized, signed target/Rastera.app
./tools/build-app.sh --install       # update existing /Applications or ~/Applications copy; otherwise ~/Applications
./tools/build-app.sh --install --destination "$HOME/Applications/Rastera.app"
open "$HOME/Applications/Rastera.app"
# Or double-click Rastera in Finder / choose it in Launchpad.
```

There is one packager behind build-app.sh. It builds/validates a fresh staged release bundle, then optionally copies it to a same-filesystem installation staging directory. Before installed replacement it requests normal quit of running Rastera development bundles and waits for shutdown/recovery flush. It never force-kills Rastera. The previous installed bundle is retained until the new signature passes; failure rolls back. No projects, preferences, recovery journals or launcher databases are replaced. Explicit destinations are allowed; non-writable protected destinations fail with an error rather than invoking sudo. A single-app Launch Services registration follows installation; no launcher layout reset is performed.

Bundle identity org.rastera.desktop.dev, stable identifier-only ad-hoc designated requirement, camera usage text, Continuity discovery and camera entitlement remain. No microphone entitlement or icon asset is added. Resources include bundled font/media notices; font bytes and shader sources are compiled into the executable. Project media remain normal external references, not copied into the app. Verification checks arm64, Info.plist, bundled notices, system-only dylib/framework dependencies, camera entitlement and strict installed signature. Normal launch needs neither Cargo nor a repository cwd/development variables.

This is local ad-hoc development signing, not Developer ID distribution or notarization. The certificate-identity override is retained but not claimed tested. The release build keeps optimization and disables optional stripping: the installed pinned Rust toolchain's rust-objcopy stalled during stripping on this Mac. No compiler/library versions changed. The previous in-place packaging also encountered Finder custom-icon resource-fork metadata in target/Rastera.app; fresh staged packaging avoids inheriting it without changing macOS security settings.

Native scripts generate synthetic media outside the bundle. Installed diagnostic probes may receive arguments, but Finder/Launchpad no-argument launches are verified separately. Existing normal recovery data must migrate, never be replaced with probe fixtures.


## Measured verification

Required formatting, locked workspace check/tests/Clippy and optimized release build pass on Apple M1 Pro/macOS 15.5 with unchanged Rust 1.99.1 and dependencies. There are 62 core tests, eight destination-test executions and one desktop test (71 Rust executions), plus 12 Python tests. Added tests cover projective inverse/grid agreement, perspective interpolation on both sides of the triangle diagonal, crop invariance, warped hit testing, off-canvas movement/anchored resize, invalid geometry, shared-source pin independence, scene/history/migration/recovery, and transactional installer rollback/preservation.

```sh
cargo fmt --all -- --check
cargo check --workspace --locked
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo build --release --workspace --locked
python3 -m unittest discover -s tools -p 'test_*.py'
./tools/build-app.sh --install
python3 tools/corner_pin_probe.py --app "$HOME/Applications/Rastera.app" --out /tmp/rastera-pin-review --capture
```

The self-contained numbered grid/PNG/text/MP4/GIF fixtures are generated outside the app (ffmpeg is test tooling only). The installed diagnostic executable runs with cwd `/` and a minimal HOME/PATH environment. Native events verify one-entry pin undo/redo, cancellation, crop after warp without point movement, whole-shape resize, independent duplicate pins, editable-text warp, scene switch/save/reopen and no prepared-resource re-upload on pin changes. Video/GIF planes are warped too. Installed snapshots preserve pinned scene geometry. No camera images/screenshots are committed.

After the user connected an external display, native read-back confirmed the Right borderless fullscreen frame equals the target NSScreen frame `(1800,89,1920,1080)` points, drawable `1920×1080`, level 26, non-key and mouse-ignored. Left remains windowed with the original exact default split. An initial assertion incorrectly compared integer JSON frame numbers to floating-point JSON values; numerical conversion fixed the test, with no presentation change. Earlier pre-connection runs had no external display and do not count as external verification.

Owned clean-window screenshots sample the original warped grid across the output seam: **11 Left samples max channel delta 1, nine Right samples delta 0**, against core forward-mapped fixture coordinates after converting capture ICC profiles to sRGB. Raw Color LCD values differed by up to 29 levels; this was display-profile encoding, not a geometry failure. Both triangles/halves show continuous grid structure; selection labels/handles remain absent from clean pictures while operator selection is active. Screenshots are sparse static geometry evidence and can perturb presentation; an uncaptured paired native replay also passes. Capture request/return times are bounds, not exact displayed-frame or freeze durations.

Camera checks use the installed bundle and an isolated saved capture project: FaceTime HD Camera 1280×720 BGRA @30 and OBS Virtual Camera 1920×1080 BGRA @60 deliver frames and upload to the producer after fresh-process reopen, with no new authorization request. OBS is virtual, not evidence of another physical camera or optical motion. No repository cwd/developer variables are used by the installed process. Normal no-argument startup separately restores the user's real existing profile (project/scene identities preserved, schema 9→10, enabled outputs remain zero); diagnostic fixtures do not replace that profile.

Finder Open and Launchpad search/tile launch both start `/Users/tdelvecc/Applications/Rastera.app/Contents/MacOS/Rastera` with no application arguments. macOS 15.5 uses **Launchpad**, not the newer Apps launcher. Its Springboard did not expose icon/search AX elements here; Dock-targeted search text and an actual click on the verified single Rastera result exercised the launcher. No launcher databases, icon layout, OS display modes or accessibility settings were reset. Actual Command-Q normally terminates the installed process; subsequent normal launches use new process IDs. Strict installed signature/DR, entitlement, arm64 architecture, system dependencies and separate-copy inode checks pass. Initial install found no running review instance; the installer nevertheless normal-quits/waits before each replacement.

Owned diagnostic shutdowns report capture drained=1 and 0/4 canvas leases. These are finite trials, not a general continuity or production claim. Human interaction/picture acceptance of new pin controls, long operation, display unplug/reconnect, more than one external, calibration, negative-origin layouts and physical synchronization remain pending. Existing operator transition pauses, zero/missing presentation evidence and capture-related limitations remain. No app-icon or broader UI overhaul is included.

References: [Apple designated requirements](https://developer.apple.com/documentation/technotes/tn3127-inside-code-signing-requirements), [code-signing bundle resource rules](https://developer.apple.com/library/archive/documentation/Security/Conceptual/CodeSigningGuide/Procedures/Procedures.html), and pinned wgpu/Naga WGSL perspective interpolation behavior. Local ad-hoc verification is separate from Gatekeeper/notarized distribution readiness.

### Final installed replay and update

The actual AppKit Escape test initially failed despite direct cancellation succeeding. A main-thread, operator-window-scoped local key monitor now forwards Escape cancellation through the existing bounded pointer queue, returning the original key event for normal text-field handling. It is removed before presenter shutdown. Replaying key down/up through AppKit’s event queue now cancels the active gesture and restores its original document without a history entry. Viewport gestures use the same cancellation route.

The final optimized installed replay also passes without screenshots; all four texture slots drain. Updating while a normal Finder-launched instance was running requested and completed normal quit before replacement. The subsequent no-argument Finder launch runs the installed executable, parent launchd, cwd `/`. Original project/scene identities remain unchanged, schema is 10 and outputs remain disabled. The isolated probe recovery snapshot contains ten non-identity pins across two scenes. The normal installed app is left open for review. Generated review media/project are under `/tmp/rastera-pin-completed/`; open `corner-pin.rastera.json` to inspect that synthetic composition. These temporary references are not bundled or added to the user's normal document.

[Machine-readable sanitized results](source-corner-pin-results.json) retain screenshot request/return bounds and separate captured/uncaptured outcomes. An existing dependency, block 0.1.6, emits a future-Rust compatibility notice; present required checks pass and dependency/toolchain versions remain unchanged.
