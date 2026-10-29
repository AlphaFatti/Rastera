//! Live canvas editor; baseline executables remain separate.
#[cfg(target_os = "macos")]
#[path = "../shared_native.rs"]
mod shared_native;
#[cfg(target_os = "macos")]
fn main() -> eframe::Result {
    shared_native::run_editor()
}
#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("canvas-editor currently requires macOS 14+");
}
