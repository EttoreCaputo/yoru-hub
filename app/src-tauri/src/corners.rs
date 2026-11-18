//! The main window's rounded corners. It has no system title bar (the UI
//! draws its own), and a window without one gets square corners on macOS:
//! its view is rounded and clipped here instead, with the system's shadow
//! following the curve. Square again in full screen, like any Mac app.

use tauri::WebviewWindow;

/// Close to a macOS window with a sidebar.
#[cfg(target_os = "macos")]
const RADIUS: f64 = 16.0;

#[cfg(target_os = "macos")]
pub fn apply(window: &WebviewWindow) {
    use objc2::msg_send;
    use objc2::runtime::AnyObject;
    use objc2_app_kit::{NSColor, NSWindow};

    let full = window.is_fullscreen().unwrap_or(false);
    let Ok(ptr) = window.ns_window() else { return };
    // SAFETY: Tauri hands over its live NSWindow; window events and setup
    // run on the main thread.
    let Some(ns) = (unsafe { (ptr as *mut NSWindow).as_ref() }) else { return };
    // Clear behind the view, so the cut corners show what's under the window.
    ns.setOpaque(false);
    ns.setBackgroundColor(Some(&NSColor::clearColor()));
    let Some(view) = ns.contentView() else { return };
    view.setWantsLayer(true);
    // SAFETY: a layer-backed view's layer is a CALayer; both setters exist on it.
    unsafe {
        let layer: *mut AnyObject = msg_send![&*view, layer];
        if layer.is_null() {
            return;
        }
        let _: () = msg_send![layer, setCornerRadius: if full { 0.0 } else { RADIUS }];
        let _: () = msg_send![layer, setMasksToBounds: true];
    }
    ns.setHasShadow(true);
    ns.invalidateShadow();
}

#[cfg(not(target_os = "macos"))]
pub fn apply(_window: &WebviewWindow) {}
