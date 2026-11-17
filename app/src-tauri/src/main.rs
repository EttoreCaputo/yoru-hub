// No console window next to the app in Windows release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use yoru_core::mcp;

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
        _ => {}
    }
    yoru_app_lib::run();
}
