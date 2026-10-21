//! Yoru in Claude Code's config files: [`mcpconfig`] adds Yoru's MCP server
//! to Claude Code, and [`cleanup`] takes out the hooks and the status line
//! older versions of the app installed, when Yoru Hub still followed Claude
//! Code's sessions.

pub mod cleanup;
pub mod mcpconfig;

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

/// Where Claude Code keeps its settings.
fn config_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("CLAUDE_CONFIG_DIR").filter(|d| !d.is_empty()) {
        return Some(PathBuf::from(dir));
    }
    crate::paths::home_dir().map(|h| h.join(".claude"))
}

fn read_settings(path: &Path) -> Option<Map<String, Value>> {
    parse_object(&fs::read_to_string(path).ok()?).ok()
}

fn parse_object(text: &str) -> io::Result<Map<String, Value>> {
    if text.trim().is_empty() {
        return Ok(Map::new());
    }
    match serde_json::from_str(text) {
        Ok(Value::Object(map)) => Ok(map),
        Ok(_) => Err(io::Error::other("the Claude Code config is not a JSON object")),
        Err(e) => Err(io::Error::other(format!("the Claude Code config is not valid JSON ({e}); fix it first"))),
    }
}

fn write_settings(path: &Path, settings: &Map<String, Value>) -> io::Result<()> {
    let mut text = serde_json::to_string_pretty(settings)?;
    text.push('\n');
    write_atomic(path, text.as_bytes())
}

/// The file as it was before Yoru first touched it.
fn backup(path: &Path) -> io::Result<()> {
    let backup = path.with_extension("json.yoru-backup");
    if path.exists() && !backup.exists() {
        fs::copy(path, backup)?;
    }
    Ok(())
}

fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension(format!("tmp{}", std::process::id()));
    fs::write(&tmp, bytes)?;
    fs::rename(&tmp, path)
}
