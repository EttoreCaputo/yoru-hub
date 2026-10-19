use std::path::Path;

/// Boards whose firmware the app embeds, as folders under `firmware/`.
const BOARDS: [&str; 1] = ["yoru"];

fn main() {
    check_firmware();
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
