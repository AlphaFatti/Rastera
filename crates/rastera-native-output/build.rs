use std::{env, path::PathBuf, process::Command};
fn run(command: &mut Command) {
    let status = command.status().expect("Apple Command Line Tools required");
    assert!(status.success(), "native bridge build failed: {command:?}");
}
fn main() {
    println!("cargo:rerun-if-changed=native/native_output.mm");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos") {
        return;
    }
    let out = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    let object = out.join("native_output.o");
    run(Command::new("xcrun")
        .args([
            "--sdk",
            "macosx",
            "clang++",
            "-std=c++17",
            "-fobjc-arc",
            "-fblocks",
            "-mmacosx-version-min=14.0",
            "-Wall",
            "-Wextra",
            "-Werror",
            "-Wno-unused-parameter",
        ])
        .arg(if env::var("PROFILE").as_deref() == Ok("release") {
            "-O3"
        } else {
            "-O0"
        })
        .args(["-c", "native/native_output.mm", "-o"])
        .arg(&object));
    run(Command::new("ar")
        .arg("crs")
        .arg(out.join("librastera_native_output.a"))
        .arg(object));
    println!("cargo:rustc-link-search=native={}", out.display());
    println!("cargo:rustc-link-lib=static=rastera_native_output");
    for framework in [
        "AppKit",
        "QuartzCore",
        "Metal",
        "Foundation",
        "CoreFoundation",
    ] {
        println!("cargo:rustc-link-lib=framework={framework}");
    }
    println!("cargo:rustc-link-lib=dylib=c++");
    println!("cargo:rustc-link-arg=-mmacosx-version-min=14.0");
}
