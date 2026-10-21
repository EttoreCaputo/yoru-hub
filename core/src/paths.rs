//! Where Yoru keeps its files on this computer.

use std::path::PathBuf;

/// Yoru's own folder: the timers, the models, the keys, the connectors.
pub(crate) fn data_dir() -> Option<PathBuf> {
    let env = |k: &str| std::env::var_os(k).filter(|v| !v.is_empty()).map(PathBuf::from);
    let base = if cfg!(target_os = "macos") {
        home_dir().map(|h| h.join("Library/Application Support"))
    } else if cfg!(windows) {
        env("APPDATA")
    } else {
        env("XDG_CONFIG_HOME").or_else(|| home_dir().map(|h| h.join(".config")))
    };
    base.map(|b| b.join("yoru-hub"))
}

pub(crate) fn home_dir() -> Option<PathBuf> {
    std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).filter(|h| !h.is_empty()).map(PathBuf::from)
}
