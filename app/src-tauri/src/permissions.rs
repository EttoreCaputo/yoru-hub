//! The Mac's permissions Yoru uses, read and asked by the app itself.
//!
//! They used to be asked by `yoru-audio`, a helper process: macOS showed no
//! prompt for it, and the click that should ask did nothing. Asked from the
//! app's own process, the prompt names Yoru and comes up in front; the
//! helper runs as part of the app, so what is granted here holds for it.
//!
//! The microphone is required (the first run and a gate in the window wait
//! for it); the calendar and the system audio are for meetings only.
//!
//! Each permission is `granted`, `denied`, `restricted`, `unknown` (never
//! asked) or `unsupported` (not on this system).

use serde::Serialize;

/// The app's bundle id, for `tccutil`.
const BUNDLE_ID: &str = "it.bytebusters.yoru.hub";

#[derive(Serialize, Clone)]
pub struct Permissions {
    microphone: String,
    calendar: String,
    system_audio: String,
}

/// Where each permission stands.
#[tauri::command]
pub fn permissions() -> Permissions {
    Permissions { microphone: status("microphone"), calendar: status("calendar"), system_audio: status("system_audio") }
}

/// Ask for one permission and say where it stands after.
///
/// macOS prompts only once per app. A permission turned down (or left in a
/// state that's neither: a record of an older build, a prompt never seen)
/// is forgotten first, so the click brings the prompt back instead of
/// sending the user to a System Settings list Yoru may not be on: macOS
/// lists an app there only once it has answered a prompt, and for the
/// system audio not always then.
#[tauri::command]
pub async fn request_permission(kind: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let before = status(&kind);
        log(&format!("{kind}: {before}, asking (macOS {})", os_version()));
        if before == "granted" || before == "restricted" || before == "unsupported" {
            return Ok(before);
        }
        if before == "denied" {
            reset(&kind);
        }
        ask(&kind)?;
        if status(&kind) == "unknown" {
            log(&format!("{kind}: no answer, forgetting the old one and asking again"));
            reset(&kind);
            ask(&kind)?;
        }
        let after = status(&kind);
        log(&format!("{kind}: now {after}"));
        Ok(after)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// In `yoru.log` at launch: where each permission stands, so a Mac where a
/// prompt never comes up can be told apart from one where it was refused.
pub fn log_state() {
    let p = permissions();
    log(&format!("macOS {}: microphone {}, calendar {}, system audio {}", os_version(), p.microphone, p.calendar, p.system_audio));
}

/// System Settings at one permission's page, for a permission turned down.
#[tauri::command]
pub fn open_privacy(kind: String) -> Result<(), String> {
    let pane = match kind.as_str() {
        "microphone" => "Privacy_Microphone",
        "calendar" => "Privacy_Calendars",
        "system_audio" => "Privacy_AudioCapture",
        _ => return Err(format!("no permission called {kind}")),
    };
    std::process::Command::new("open")
        .arg(format!("x-apple.systempreferences:com.apple.preference.security?{pane}"))
        .status()
        .map_err(|e| e.to_string())
        .map(|_| ())
}

/// In `yoru.log`: when a prompt doesn't come up, what macOS said.
fn log(line: &str) {
    yoru_core::hostlog::line("permissions", line);
}

/// Forget this app's answer for one permission (only Yoru's, nobody else's).
fn reset(kind: &str) {
    let service = match kind {
        "microphone" => "Microphone",
        "calendar" => "Calendar",
        "system_audio" => "AudioCapture",
        _ => return,
    };
    match std::process::Command::new("tccutil").args(["reset", service, BUNDLE_ID]).output() {
        Ok(out) if out.status.success() => {}
        Ok(out) => log(&format!("{kind}: tccutil: {}", String::from_utf8_lossy(&out.stderr).trim())),
        Err(e) => log(&format!("{kind}: tccutil: {e}")),
    }
}

/// The macOS version, as `14.2`; empty elsewhere. Read once: the window
/// asks for the permissions every second.
fn os_version() -> &'static str {
    static VERSION: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    VERSION.get_or_init(|| {
        std::process::Command::new("sw_vers")
            .arg("-productVersion")
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .unwrap_or_default()
    })
}

#[cfg(target_os = "macos")]
fn status(kind: &str) -> String {
    match kind {
        "microphone" => mac::microphone(),
        "calendar" => mac::calendar(),
        // Core Audio taps and their permission came with macOS 14.2; before,
        // TCC takes the service for an unknown one and says denied.
        "system_audio" if !at_least(os_version(), (14, 2)) => "unsupported",
        "system_audio" => mac::system_audio(),
        _ => "unsupported",
    }
    .to_string()
}

/// Whether a version like `14.2.1` is at least `(major, minor)`.
#[cfg(target_os = "macos")]
fn at_least(version: &str, (major, minor): (u32, u32)) -> bool {
    let mut parts = version.split('.').map(|p| p.parse::<u32>().unwrap_or(0));
    let (a, b) = (parts.next().unwrap_or(0), parts.next().unwrap_or(0));
    (a, b) >= (major, minor)
}

#[cfg(target_os = "macos")]
fn ask(kind: &str) -> Result<(), String> {
    match kind {
        "microphone" => mac::ask_microphone(),
        "calendar" => mac::ask_calendar(),
        "system_audio" => mac::ask_system_audio(),
        _ => return Err(format!("no permission called {kind}")),
    }
    Ok(())
}

// Elsewhere nothing is asked: the window lets everything through.
#[cfg(not(target_os = "macos"))]
fn status(_kind: &str) -> String {
    "granted".into()
}

#[cfg(not(target_os = "macos"))]
fn ask(_kind: &str) -> Result<(), String> {
    Ok(())
}

#[cfg(target_os = "macos")]
mod mac {
    use std::ffi::{c_char, c_int, c_void};
    use std::sync::mpsc;
    use std::time::Duration;

    /// A prompt left unanswered this long lets the button go; macOS keeps
    /// it up, and the list reads the answer when it comes.
    const WAIT: Duration = Duration::from_secs(120);

    use block2::RcBlock;
    use objc2::runtime::Bool;
    use objc2_av_foundation::{AVAuthorizationStatus, AVCaptureDevice, AVMediaTypeAudio};
    use objc2_event_kit::{EKAuthorizationStatus, EKEntityType, EKEventStore};
    use objc2_foundation::{NSError, NSString};

    fn av_name(status: AVAuthorizationStatus) -> &'static str {
        match status {
            AVAuthorizationStatus::Authorized => "granted",
            AVAuthorizationStatus::Denied => "denied",
            AVAuthorizationStatus::Restricted => "restricted",
            _ => "unknown",
        }
    }

    pub fn microphone() -> &'static str {
        let Some(audio) = (unsafe { AVMediaTypeAudio }) else { return "unsupported" };
        av_name(unsafe { AVCaptureDevice::authorizationStatusForMediaType(audio) })
    }

    /// Blocks until the user answers macOS's prompt.
    pub fn ask_microphone() {
        let Some(audio) = (unsafe { AVMediaTypeAudio }) else { return };
        let (tx, rx) = mpsc::channel();
        let done = RcBlock::new(move |_: Bool| {
            let _ = tx.send(());
        });
        unsafe { AVCaptureDevice::requestAccessForMediaType_completionHandler(audio, &done) };
        let _ = rx.recv_timeout(WAIT);
    }

    pub fn calendar() -> &'static str {
        match unsafe { EKEventStore::authorizationStatusForEntityType(EKEntityType::Event) } {
            EKAuthorizationStatus::FullAccess => "granted",
            // Write-only access can't read when a meeting starts.
            EKAuthorizationStatus::Denied | EKAuthorizationStatus::WriteOnly => "denied",
            EKAuthorizationStatus::Restricted => "restricted",
            _ => "unknown",
        }
    }

    pub fn ask_calendar() {
        let store = unsafe { EKEventStore::new() };
        let (tx, rx) = mpsc::channel();
        let done = RcBlock::new(move |_: Bool, _: *mut NSError| {
            let _ = tx.send(());
        });
        unsafe { store.requestFullAccessToEventsWithCompletion(RcBlock::as_ptr(&done)) };
        let _ = rx.recv_timeout(WAIT);
    }

    // The system audio (Core Audio taps) has no public call to read or ask
    // for its permission; the TCC framework's own does both, as Apple's
    // sample code for taps and the apps built on it do. Looked up at run
    // time: if a later macOS drops it, the permission reads as unsupported
    // and the tap asks for it the first time a meeting is recorded.
    type Preflight = unsafe extern "C" fn(*const c_void, *const c_void) -> c_int;
    type Request = unsafe extern "C" fn(*const c_void, *const c_void, *const c_void);

    const SERVICE: &str = "kTCCServiceAudioCapture";

    fn tcc(symbol: &[u8]) -> Option<*mut c_void> {
        unsafe {
            let lib = libc::dlopen(c"/System/Library/PrivateFrameworks/TCC.framework/Versions/A/TCC".as_ptr(), libc::RTLD_NOW);
            if lib.is_null() {
                return None;
            }
            let f = libc::dlsym(lib, symbol.as_ptr() as *const c_char);
            (!f.is_null()).then_some(f)
        }
    }

    pub fn system_audio() -> &'static str {
        let Some(f) = tcc(b"TCCAccessPreflight\0") else { return "unsupported" };
        let preflight: Preflight = unsafe { std::mem::transmute(f) };
        let service = NSString::from_str(SERVICE);
        // 0 granted, 1 denied, 2 never asked.
        match unsafe { preflight(&*service as *const NSString as *const c_void, std::ptr::null()) } {
            0 => "granted",
            1 => "denied",
            _ => "unknown",
        }
    }

    pub fn ask_system_audio() {
        let Some(f) = tcc(b"TCCAccessRequest\0") else { return };
        let request: Request = unsafe { std::mem::transmute(f) };
        let service = NSString::from_str(SERVICE);
        let (tx, rx) = mpsc::channel();
        let done = RcBlock::new(move |_: Bool| {
            let _ = tx.send(());
        });
        unsafe { request(&*service as *const NSString as *const c_void, std::ptr::null(), RcBlock::as_ptr(&done) as *const c_void) };
        let _ = rx.recv_timeout(WAIT);
    }
}
