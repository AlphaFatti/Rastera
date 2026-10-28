# Rastera

Rastera is a free, open-source live video and presentation application in early development, targeting Apple silicon Macs first. A possible Windows port is future work. The ambition is a flexible live production canvas, output routing, and presentation workflow; the current application is only a GPU canvas prototype.

The long-term [architecture blueprint](docs/architecture.md) describes planned work, not implemented capabilities. See [development status](docs/development-status.md) for the current scope and verification record. An application source license still needs to be selected before a public release.

## WIP checkpoint — acceptance pending

The minimize/restore transition stall remains unresolved. Automated formatting, compilation/build, tests, and Clippy pass. Individual native Operator, Left, and Right minimize/restore transitions were exercised and recovered. The stricter all-window automation check remains unverified after a System Events timeout. This checkpoint does not signify first-canvas milestone acceptance; see the detailed verification record in [development status](docs/development-status.md).

## Setup on an Apple silicon Mac

Install Apple Command Line Tools if needed:

```sh
xcode-select --install
```

Install Rust through the official installer (https://rustup.rs):

```sh
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs -o /tmp/rustup-init.sh
sh /tmp/rustup-init.sh -y --profile minimal
source "$HOME/.cargo/env"
```

From the repository root, `rust-toolchain.toml` selects Rust 1.99.1 and its formatting/lint components. Cargo downloads dependencies on the first build. Full Xcode was not required for the local prototype build; the minimum supported macOS version is not yet established.

## Build and run

```sh
cargo build --workspace --locked
cargo run --locked -p rastera-desktop

# Or open both projection windows immediately:
cargo run --locked -p rastera-desktop -- --outputs
```

Enable **Left projection** and **Right projection** in the operator window to open independent native windows. Closing an output clears its checkbox; enable it again to reopen. Close the operator window to quit. Output windows contain only the image and black aspect-fit bars, inside ordinary native window chrome.

The preview shows the full 3840 × 1080 GPU texture. Left selects `(0, 0, 1920, 1080)` and right selects `(1920, 0, 1920, 1080)`, using half-open pixel bounds. The blue half has one white identification block; the orange half has two. Both have a 120-pixel grid and a moving yellow marker. Green and magenta single-pixel seam columns distinguish pixels 1919 and 1920. Resize any window without changing the canvas or slices.

## Validation

```sh
cargo fmt --all -- --check
cargo check --workspace --locked
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
```

`rastera-core` owns geometry and the UI-independent wgpu renderer; `rastera-desktop` uses eframe/egui for the native operator and projection viewports. eframe 0.31.1 and wgpu 24.0.5 deliberately use the same wgpu generation; Cargo.lock records the resolved dependency graph. This is a bounded experiment with an older compatible API pair, not a claim that these are the latest releases. Dependency API reference: [eframe](https://docs.rs/eframe/0.31.1/eframe/) and [wgpu](https://docs.rs/wgpu/24.0.5/wgpu/).

## Scheduling and boundaries

A dedicated producer thread uses a monotonic `Instant` clock and a target 60 Hz deadline. It renders one fixed-size RGBA8Unorm texture, waits for its own GPU submission to complete before producing another frame, and skips missed deadlines rather than catching up in a burst. Marker position derives from elapsed time. Rendering is reusable without egui and continues while the operator is idle. No CPU pixel readback is used.

All three views register/sample the same texture on the same wgpu device/queue; UV rectangles select the slices. Queue ordering serializes GPU writes and reads. After completing a frame, the producer notifies the operator and each enabled output independently. Outputs use deferred egui viewports, so repainting an output does not require an operator update. Pending repaint requests are coalesced; a minimized viewport does not accumulate a new repaint cause on every producer frame. egui may perform two UI passes per notification. Views sample the latest available texture through eframe's event loop. There is no per-window frame queue or guarantee that different windows present the same frame simultaneously. Slow GPU work can delay all views on this shared queue. The producer's GPU wait bounds its outstanding submissions but is not a real-time scheduler; OS sleep/App Nap and display refresh may affect pacing. UI responsiveness still affects presentation, though it does not supply the producer clock.

This is an opaque SDR diagnostic pattern, not a calibrated media color pipeline. Projection uses nearest sampling and aspect-fit mapping; physical-pixel alignment at arbitrary Retina scales/window sizes needs visual verification. There is no fullscreen display assignment, genlock, calibrated frame timing, persistence, editor, media decoding, capture, audio, NDI, DeckLink, or slide engine yet. Minimize/restore visual acceptance remains pending; see the regression findings in the development-status document.

## Minimize/restore diagnostics

```sh
cargo run --locked -p rastera-desktop -- --outputs --diagnostics
```

Once per second, stderr reports monotonic timestamps, producer submissions/completions and phase (`0` pacing, `1` recording, `2` queue submission, `3` GPU completion wait, `4` notifying viewports, `5` encoder finish). Fresh interval samples time recording, encoder finish, queue submission, completion wait and notification separately, plus producer cadence. Each reports count, total, maximum and the maximum's start timestamp. Per-viewport intervals report UI entry/return counts, CPU paint-pass entries and callback gaps with start timestamps. These are CPU-side diagnostics, not GPU execution or display-presentation timestamps. Counters are sampled independently and can differ briefly while a frame is in progress. The reported minimized flag is egui's cached state: egui-winit 0.31.1 does not refresh it from native macOS window state on every frame, so it may remain false after a native minimize.

An opt-in native-command driver exercises three minimize/restore cycles and closes the operator afterward. Run each case separately:

```sh
cargo run --locked -p rastera-desktop -- --minimize-test=operator
cargo run --locked -p rastera-desktop -- --minimize-test=left
cargo run --locked -p rastera-desktop -- --minimize-test=right
cargo run --locked -p rastera-desktop -- --minimize-test=all
```

This driver sends native window commands through egui. It does not substitute for clicking macOS window controls and observing the visible outputs during the transition. AppKit's synchronous minimize/restore command itself may pause the UI thread during the animation.

## Presentation controls

See [the controlled presentation investigation](docs/presentation-investigation.md) for measured results and reproducible native-control/egui-driver commands. The framework-only control also reproduces the transition pause; the shared canvas is not necessary for that effect. The full egui-driven restore additionally stalls producer queue submission. Milestone acceptance remains pending.

```sh
cargo run --locked -p rastera-core --bin headless-producer -- --seconds=24
cargo run --locked -p rastera-desktop --bin framework-control
```

## Independent native output experiment (macOS 14+)

The separate presentation control retains an eframe operator and renders its own diagnostic pattern through CAMetalDisplayLink/Metal, outside eframe repaint callbacks. It does not use Rastera's shared canvas. See [native output investigation](docs/native-output-investigation.md) for matched release measurements, native-bridge dependencies, remaining timing gaps and limitations. Integrated output and first-canvas acceptance remain pending.

```sh
cargo run --release --locked -p rastera-native-output --bin native-output-control
```

## Shared canvas with native clean outputs (WIP, macOS 14+)

```sh
cargo run --release --locked -p rastera-desktop --bin shared-canvas-native
# Staged bring-up: native full preview only, then Left, then both outputs:
cargo run --release --locked -p rastera-desktop --bin shared-canvas-native -- --outputs=0
cargo run --release --locked -p rastera-desktop --bin shared-canvas-native -- --outputs=1
cargo run --release --locked -p rastera-desktop --bin shared-canvas-native -- --outputs=2
```

**Operator / UI preview** contains controls and the full native canvas preview. **Native clean output — Left/Right** are the actual 1920 × 1080 halves. Operator checkboxes close/reopen outputs; closing the operator quits. Arrange all windows without overlap for visual checks. One core producer publishes completed frames into a fixed four-texture GPU pool; native consumers retain each texture until their GPU reads finish. No CPU frame readback is used.

The operator UI transition pause remains. Clean-output movement was captured through transitions, but capturing the composited operator can still stall native callbacks; presentation/capture limitations and unverified hardware checks are recorded in [shared-canvas integration](docs/shared-canvas-integration.md). Acceptance remains pending. The original application and framework control commands above remain available. The independent `native-output-control` windows now end in `(control)` and use explicit role titles; that baseline still renders its own diagnostic pattern.
