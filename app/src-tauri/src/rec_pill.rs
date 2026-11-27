//! The recording pill (docs/meetings.md, Recording): while a meeting
//! records, a small window just under the menu bar, centred on the notch
//! (else on the main screen), says for how long and has Stop. It shows
//! whatever started the recording (the app, the board, the detection) and
//! whether or not Yoru is in the notch: a recording should never run
//! unseen. The page is `app/src/RecordingPill.svelte`.

use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};

pub const LABEL: &str = "rec-pill";
/// The window: the pill and room for its shadow, in points.
const WIDTH: f64 = 200.0;
const HEIGHT: f64 = 72.0;

/// A recording started: show the pill.
pub fn show(app: &AppHandle) {
    if app.get_webview_window(LABEL).is_some() {
        return;
    }
    let built = WebviewWindowBuilder::new(app, LABEL, WebviewUrl::App("index.html".into()))
        .initialization_script("window.__YORU_PILL__ = true;")
        .title("Yoru")
        .inner_size(WIDTH, HEIGHT)
        .decorations(false)
        .transparent(true)
        .background_color(tauri::window::Color(0, 0, 0, 0))
        .shadow(false)
        .resizable(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .visible_on_all_workspaces(true)
        .focused(false)
        // Stop works on the first click, without making the window key first.
        .accept_first_mouse(true)
        .visible(false)
        .build();
    let window = match built {
        Ok(w) => w,
        Err(e) => return eprintln!("yoru: the recording pill: {e}"),
    };
    let shown = window.clone();
    let _ = app.run_on_main_thread(move || {
        crate::notch::place_under_bar(&shown, WIDTH, HEIGHT);
        let _ = shown.show();
    });
}

/// The recording ended: take the pill away.
pub fn hide(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(LABEL) {
        let _ = window.close();
    }
}
