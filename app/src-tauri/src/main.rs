// No console window next to the app in Windows release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use yoru_core::mcp;

/// What older versions put in Claude Code's settings, as a hook and as the
/// status line. The app takes those out at start (`claude::cleanup`); until
/// then Claude Code may still run them, and they must end at once, silently,
/// rather than open the app.
const RETIRED_FLAGS: [&str; 2] = ["--claude-hook", "--claude-statusline"];

fn main() {
    // An MCP client runs us as its server; handle that before the
    // single-instance check would hand it to the running app.
    match std::env::args().nth(1).as_deref() {
        Some(mcp::FLAG) => {
            if let Err(e) = mcp::serve_stdio(None) {
                eprintln!("yoru mcp: {e}");
                std::process::exit(1);
            }
            return;
        }
        Some(flag) if RETIRED_FLAGS.contains(&flag) => {
            let _ = std::io::copy(&mut std::io::stdin().lock(), &mut std::io::sink());
            return;
        }
        _ => {}
    }
    yoru_app_lib::run();
}
