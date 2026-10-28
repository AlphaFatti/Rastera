//! Shared core canvas / native Metal consumers. Existing application stays a baseline.
#[cfg(target_os = "macos")]
#[path = "../shared_native.rs"]
mod shared_native;
#[cfg(target_os = "macos")]
fn main() -> eframe::Result {
    shared_native::run()
}
#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("shared-canvas-native requires macOS 14+");
}
