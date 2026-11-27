//! The Yoru in the notch (docs/notch.md): a transparent window over the
//! top edge of the screen with the camera's notch, as wide as Yoru may grow
//! there. Closed, Yoru sits beside the camera; with the pointer on him he
//! opens into a panel. Clicks go to the apps underneath everywhere but on
//! what the page draws: it says where that is ([`set_content`]), and a
//! thread here follows the pointer, since a window that lets clicks through
//! hears nothing of the mouse.
//!
//! macOS keeps ordinary windows below the menu bar; this one is put above
//! it, on every Space and over full-screen apps, through AppKit itself.

use std::sync::Mutex;
use std::thread;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

pub const LABEL: &str = "notch";
/// The window: the most the panel takes, in points.
const WIDTH: f64 = 620.0;
const HEIGHT: f64 = 520.0;
/// How often the pointer is looked at.
const POINTER_EVERY: Duration = Duration::from_millis(40);
/// How often the host hears the user may talk, while the pointer is on the
/// notch (Gemini Live keeps its session open ahead).
const POINTER_ON_EVERY: Duration = Duration::from_secs(5);
/// Room around what the page draws that still counts as on it.
const SLACK: f64 = 8.0;

/// Where the notch is, for the page, in points of the window.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Geometry {
    /// False on a screen without one: the page draws its own.
    pub notch: bool,
    /// The camera's notch: its width and the menu bar's height.
    pub width: f64,
    pub height: f64,
    /// The window's width, the notch being at its middle.
    pub window: f64,
}

/// What the page draws, in points of the window.
#[derive(Debug, Clone, Copy, Default, Deserialize)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl Rect {
    fn holds(&self, x: f64, y: f64) -> bool {
        x >= self.x - SLACK && x <= self.x + self.w + SLACK && y >= self.y - SLACK && y <= self.y + self.h + SLACK
    }
}

/// The notch window's state on the Rust side.
#[derive(Default)]
pub struct Notch {
    content: Mutex<Rect>,
    geometry: Mutex<Option<Geometry>>,
}

/// The page's drawing moved or changed size.
#[tauri::command]
pub fn notch_content(app: AppHandle, rect: Rect) {
    *app.state::<Notch>().content.lock().unwrap() = rect;
}

/// The notch's geometry, for a page that loaded after it was sent.
#[tauri::command]
pub fn notch_geometry(app: AppHandle) -> Option<Geometry> {
    *app.state::<Notch>().geometry.lock().unwrap()
}

/// Hold ⌥Space to talk to the Yoru in the notch.
pub const TALK_KEY: &str = "Alt+Space";

/// Hand the notch's events to its window, and the talk key to the core.
pub fn plugin() -> tauri::plugin::TauriPlugin<tauri::Wry> {
    use tauri_plugin_global_shortcut::{Builder, ShortcutState};
    Builder::new()
        .with_handler(|_, _, event| yoru_core::notch::talk(event.state() == ShortcutState::Pressed))
        .build()
}

/// Turn the Yoru in the notch on or off: his window and his key. His
/// session, the Mac's ears and voice, runs either way: the window's Home
/// talks through it too, and shows what it does.
pub fn apply(app: &AppHandle, on: bool) {
    use tauri_plugin_global_shortcut::GlobalShortcutExt;
    yoru_core::notch::set_enabled(true);
    let events = app.clone();
    yoru_core::notch::on_event(move |e| {
        let _ = events.emit("notch", e);
    });
    let shortcuts = app.global_shortcut();
    let registered = shortcuts.is_registered(TALK_KEY);
    if on && !registered {
        // Another app may hold it; the button in the panel still works.
        if let Err(e) = shortcuts.register(TALK_KEY) {
            eprintln!("yoru: the notch's talk key: {e}");
        }
    } else if !on && registered {
        let _ = shortcuts.unregister(TALK_KEY);
    }
    set_shown(app, on);
}

/// Push-to-talk from the panel's button.
#[tauri::command]
pub fn notch_talk(pressed: bool) {
    yoru_core::notch::talk(pressed);
}

/// The notch closed: let go of the Mac's audio.
#[tauri::command]
pub fn notch_release() {
    yoru_core::notch::release();
}

/// Stop what Yoru says or hears.
#[tauri::command]
pub fn notch_hush() {
    yoru_core::notch::hush();
}

/// Show Yoru in the notch, or take him away.
pub fn set_shown(app: &AppHandle, shown: bool) {
    match (shown, app.get_webview_window(LABEL)) {
        (true, None) => {
            if let Err(e) = open(app) {
                eprintln!("yoru: the notch window: {e}");
            }
        }
        (false, Some(window)) => {
            let _ = window.close();
        }
        _ => {}
    }
}

fn open(app: &AppHandle) -> tauri::Result<()> {
    let window = WebviewWindowBuilder::new(app, LABEL, WebviewUrl::App("index.html".into()))
        // The bundle mounts the notch for it; a query in the URL would be
        // taken for part of the file's name.
        .initialization_script("window.__YORU_NOTCH__ = true;")
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
        .visible(false)
        .build()?;
    let _ = window.set_ignore_cursor_events(true);
    place(app, &window);
    let _ = window.show();
    follow_pointer(app.clone());
    Ok(())
}

/// Put the window at the top of the screen with the notch (else the main
/// screen), above the menu bar, and tell the page where the notch is.
pub fn place(app: &AppHandle, window: &WebviewWindow) {
    let (app, window) = (app.clone(), window.clone());
    let main = app.clone();
    let _ = main.run_on_main_thread(move || {
        let Some(geometry) = mac::place(&window, WIDTH, HEIGHT) else { return };
        *app.state::<Notch>().geometry.lock().unwrap() = Some(geometry);
        let _ = window.emit("notch-geometry", geometry);
    });
}

/// Put `window` just under the menu bar, centred on the notch (else on the
/// main screen), below the notch's own window: the recording pill.
/// On the main thread.
pub fn place_under_bar(window: &WebviewWindow, width: f64, height: f64) {
    mac::place_under_bar(window, width, height);
}

/// Let clicks through but on what the page draws, and tell it when the
/// pointer comes and goes. Ends with the window.
fn follow_pointer(app: AppHandle) {
    thread::spawn(move || {
        let mut over = false;
        let mut last_on = None;
        let mut screens = mac::screens_signature();
        loop {
            thread::sleep(POINTER_EVERY);
            let Some(window) = app.get_webview_window(LABEL) else { return };
            // A screen plugged in, taken away or resized: place it again.
            let now = mac::screens_signature();
            if now != screens {
                screens = now;
                place(&app, &window);
            }
            let (Ok(pointer), Ok(origin), Ok(scale)) = (app.cursor_position(), window.outer_position(), window.scale_factor()) else { continue };
            let (x, y) = ((pointer.x - origin.x as f64) / scale, (pointer.y - origin.y as f64) / scale);
            let on = app.state::<Notch>().content.lock().unwrap().holds(x, y);
            if on != over {
                over = on;
                let _ = window.set_ignore_cursor_events(!on);
                let _ = window.emit("notch-pointer", on);
            }
            // Again while the pointer stays: it may have come before the
            // notch's session was up.
            if on && last_on.is_none_or(|t: Instant| t.elapsed() >= POINTER_ON_EVERY) {
                last_on = Some(Instant::now());
                yoru_core::notch::pointer_on();
            } else if !on {
                last_on = None;
            }
        }
    });
}

#[cfg(target_os = "macos")]
mod mac {
    use objc2::MainThreadMarker;
    use objc2_app_kit::{NSColor, NSScreen, NSStatusWindowLevel, NSWindow, NSWindowCollectionBehavior};
    use objc2_foundation::{NSPoint, NSRect, NSSize};
    use tauri::WebviewWindow;

    use super::Geometry;

    /// The software notch on a screen without one.
    const SOFT_WIDTH: f64 = 190.0;
    const SOFT_HEIGHT: f64 = 32.0;

    pub fn place(window: &WebviewWindow, width: f64, height: f64) -> Option<Geometry> {
        let marker = MainThreadMarker::new()?;
        let screens = NSScreen::screens(marker);
        let notched = screens.iter().find(|s| s.safeAreaInsets().top > 0.0);
        let screen = notched.or_else(|| NSScreen::mainScreen(marker))?;
        let frame = screen.frame();
        let geometry = if screen.safeAreaInsets().top > 0.0 {
            let (left, right) = (screen.auxiliaryTopLeftArea(), screen.auxiliaryTopRightArea());
            Geometry { notch: true, width: (frame.size.width - left.size.width - right.size.width).max(0.0), height: screen.safeAreaInsets().top, window: width }
        } else {
            let bar = frame.origin.y + frame.size.height - (screen.visibleFrame().origin.y + screen.visibleFrame().size.height);
            Geometry { notch: false, width: SOFT_WIDTH, height: if bar > 0.0 { bar } else { SOFT_HEIGHT }, window: width }
        };
        let ns: *mut NSWindow = window.ns_window().ok()?.cast();
        // SAFETY: Tauri hands over its live NSWindow; we're on the main thread.
        let ns = unsafe { ns.as_ref()? };
        // Clear all the way down, or it's a black box over the screen.
        ns.setOpaque(false);
        ns.setBackgroundColor(Some(&NSColor::clearColor()));
        ns.setHasShadow(false);
        ns.setLevel(NSStatusWindowLevel + 1);
        ns.setCollectionBehavior(
            NSWindowCollectionBehavior::CanJoinAllSpaces
                | NSWindowCollectionBehavior::Stationary
                | NSWindowCollectionBehavior::FullScreenAuxiliary
                | NSWindowCollectionBehavior::IgnoresCycle,
        );
        // AppKit counts up from the bottom; a borderless window set this way
        // isn't pushed below the menu bar.
        let origin = NSPoint::new(frame.origin.x + (frame.size.width - width) / 2.0, frame.origin.y + frame.size.height - height);
        ns.setFrame_display(NSRect::new(origin, NSSize::new(width, height)), true);
        Some(geometry)
    }

    /// Under the menu bar of the screen `place` picks, one level below the
    /// notch's window, so the open panel covers it.
    pub fn place_under_bar(window: &WebviewWindow, width: f64, height: f64) -> Option<()> {
        let marker = MainThreadMarker::new()?;
        let screens = NSScreen::screens(marker);
        let notched = screens.iter().find(|s| s.safeAreaInsets().top > 0.0);
        let screen = notched.or_else(|| NSScreen::mainScreen(marker))?;
        let frame = screen.frame();
        let visible = screen.visibleFrame();
        let bar = (frame.origin.y + frame.size.height - (visible.origin.y + visible.size.height)).max(screen.safeAreaInsets().top);
        let ns: *mut NSWindow = window.ns_window().ok()?.cast();
        // SAFETY: Tauri hands over its live NSWindow; we're on the main thread.
        let ns = unsafe { ns.as_ref()? };
        ns.setOpaque(false);
        ns.setBackgroundColor(Some(&NSColor::clearColor()));
        ns.setHasShadow(false);
        ns.setLevel(NSStatusWindowLevel);
        ns.setCollectionBehavior(
            NSWindowCollectionBehavior::CanJoinAllSpaces
                | NSWindowCollectionBehavior::Stationary
                | NSWindowCollectionBehavior::FullScreenAuxiliary
                | NSWindowCollectionBehavior::IgnoresCycle,
        );
        let origin = NSPoint::new(frame.origin.x + (frame.size.width - width) / 2.0, frame.origin.y + frame.size.height - bar - height);
        ns.setFrame_display(NSRect::new(origin, NSSize::new(width, height)), true);
        Some(())
    }

    /// The screens' frames, to notice when they change.
    pub fn screens_signature() -> Vec<(i64, i64, i64, i64)> {
        // NSScreen wants the main thread; CoreGraphics answers anywhere.
        use objc2_core_graphics::{CGDisplayBounds, CGGetActiveDisplayList};
        let mut ids = [0u32; 16];
        let mut count = 0u32;
        // SAFETY: the list has room for 16 displays and count says how many came.
        if unsafe { CGGetActiveDisplayList(16, ids.as_mut_ptr(), &mut count) } != objc2_core_graphics::CGError::Success {
            return Vec::new();
        }
        ids[..count as usize]
            .iter()
            .map(|&id| {
                let b = CGDisplayBounds(id);
                (b.origin.x as i64, b.origin.y as i64, b.size.width as i64, b.size.height as i64)
            })
            .collect()
    }
}

#[cfg(not(target_os = "macos"))]
mod mac {
    use tauri::WebviewWindow;

    use super::Geometry;

    pub fn place(_: &WebviewWindow, width: f64, _: f64) -> Option<Geometry> {
        Some(Geometry { notch: false, width: 190.0, height: 32.0, window: width })
    }

    pub fn screens_signature() -> Vec<(i64, i64, i64, i64)> {
        Vec::new()
    }

    pub fn place_under_bar(_: &WebviewWindow, _: f64, _: f64) -> Option<()> {
        None
    }
}
