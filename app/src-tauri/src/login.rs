//! Yoru opens at login (Settings → This Mac): the meeting detection, the
//! timers and the notch only work while the app runs, and nothing reminded
//! anyone to open it after a restart. On by default; the app registers
//! itself once as a login item (`SMAppService`, the entry in System Settings
//! → General → Login Items), and after that only the toggle changes it, so
//! turning it off in System Settings is respected.

/// Whether Yoru opens at login now, as macOS says.
pub fn enabled() -> bool {
    mac::status() == Some(true)
}

/// Add Yoru to the login items, or take it away.
pub fn set(on: bool) -> Result<(), String> {
    mac::set(on)
}

#[cfg(target_os = "macos")]
mod mac {
    use objc2_service_management::{SMAppService, SMAppServiceStatus};

    pub fn status() -> Option<bool> {
        // SAFETY: plain class and property calls, safe from any thread.
        let status = unsafe { SMAppService::mainAppService().status() };
        Some(status == SMAppServiceStatus::Enabled || status == SMAppServiceStatus::RequiresApproval)
    }

    pub fn set(on: bool) -> Result<(), String> {
        // SAFETY: as above.
        let service = unsafe { SMAppService::mainAppService() };
        let done = unsafe { if on { service.registerAndReturnError() } else { service.unregisterAndReturnError() } };
        done.map_err(|e| e.localizedDescription().to_string())
    }
}

#[cfg(not(target_os = "macos"))]
mod mac {
    pub fn status() -> Option<bool> {
        None
    }

    pub fn set(_: bool) -> Result<(), String> {
        Err("only on macOS".into())
    }
}
