use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Boards whose firmware the app embeds, as folders under `firmware/`.
const BOARDS: [&str; 1] = ["yoru"];

fn main() {
    check_firmware();
    build_audio_helper();
    tauri_build::build();
}

/// lib.rs embeds `firmware/<board>/merged-binary.bin` as it is on disk; it comes
/// from the board's own repository through `tools/fetch_firmware.sh`. Say so
/// before `include_bytes!` fails with a bare "file not found".
fn check_firmware() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../firmware");
    for board in BOARDS {
        let image = root.join(board).join("merged-binary.bin");
        println!("cargo:rerun-if-changed={}", image.display());
        if !image.exists() {
            println!("cargo:warning=no firmware/{board}/merged-binary.bin: build the firmware and run tools/fetch_firmware.sh");
        }
    }
}

/// `yoru-audio`, the Swift helper that records meetings (`audio/`), into
/// `binaries/` where `externalBin` wants it. Built here rather than by
/// `tools/build_sidecars.sh` because every build needs it: tauri-build
/// refuses an `externalBin` that isn't there, even for `cargo check`.
fn build_audio_helper() {
    let target = env::var("TARGET").unwrap();
    if !target.ends_with("-apple-darwin") {
        return;
    }
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let package = manifest.join("../../audio");
    println!("cargo:rerun-if-changed={}", package.join("Package.swift").display());
    println!("cargo:rerun-if-changed={}", package.join("Sources").display());
    let arch = if target.starts_with("aarch64") { "arm64" } else { "x86_64" };
    let scratch = PathBuf::from(env::var("OUT_DIR").unwrap()).join("audio");
    let swift = |extra: &[&str]| {
        Command::new("swift")
            .args(["build", "-c", "release", "--arch", arch, "--package-path"])
            .arg(&package)
            .arg("--scratch-path")
            .arg(&scratch)
            .args(extra)
            // Cargo's own target settings would confuse SwiftPM's compiler.
            .env_remove("SDKROOT")
            .env_remove("MACOSX_DEPLOYMENT_TARGET")
            .output()
            .unwrap_or_else(|e| panic!("yoru-audio: can't run swift ({e}); install Xcode or its command line tools"))
    };
    let built = swift(&[]);
    if !built.status.success() {
        panic!("yoru-audio: swift build failed\n{}{}", String::from_utf8_lossy(&built.stdout), String::from_utf8_lossy(&built.stderr));
    }
    let bin = PathBuf::from(String::from_utf8_lossy(&swift(&["--show-bin-path"]).stdout).trim()).join("yoru-audio");
    let dest = manifest.join("binaries");
    fs::create_dir_all(&dest).unwrap();
    fs::copy(&bin, dest.join(format!("yoru-audio-{target}"))).unwrap_or_else(|e| panic!("yoru-audio: copying {}: {e}", bin.display()));
}
