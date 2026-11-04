//! Updating a board over its link, WiFi included: the app's image goes in
//! chunks with `ota/begin`, `ota/write` and `ota/end` (Yoru's
//! `docs/protocol.md`, OTA), and the board boots it. Firmware 0.19.0 and
//! later say they can with the `ota` capability in their `hello`; a board
//! flashed before has one app slot and needs one flash over USB first.
//!
//! The board checks the image is Yoru's and its SHA-256, and goes back to
//! the firmware before if the new one doesn't reach a host within minutes.

use std::time::Duration;

use base64::Engine;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::firmware;
use crate::link::CallError;

/// What the board takes in one `ota/write` when it doesn't say.
const CHUNK: usize = 2048;
/// `ota/begin` may wait for the first flash sector's erase.
const BEGIN_TIMEOUT: Duration = Duration::from_secs(20);
/// A chunk's write, with its sector's erase.
const WRITE_TIMEOUT: Duration = Duration::from_secs(10);
/// `ota/end` checks the whole image on the board.
const END_TIMEOUT: Duration = Duration::from_secs(30);
/// Lost replies retried before giving up.
const RETRIES: usize = 3;

/// How the update is going.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Progress {
    /// Fraction of the image written, 0 to 1.
    Sent(f32),
    /// Checked and set to boot: the board restarts now.
    Restarting,
}

/// The app partition's image inside a merged flash image (bootloader,
/// partition table, app...): from the first app partition's offset to the
/// end of the image proper, its segments, checksum and appended hash.
pub fn app_image(merged: &[u8]) -> Option<&[u8]> {
    let at = firmware::app_offset(merged)?;
    let app = merged.get(at..)?;
    app.get(..image_len(app)?)
}

/// The length of an ESP-IDF app image (`esp_image_header_t` and on).
fn image_len(app: &[u8]) -> Option<usize> {
    if *app.first()? != 0xE9 {
        return None;
    }
    let segments = app[1] as usize;
    let hash_appended = *app.get(23)? == 1;
    let mut at = 24;
    for _ in 0..segments {
        let len = u32::from_le_bytes(app.get(at + 4..at + 8)?.try_into().ok()?) as usize;
        at += 8 + len;
    }
    // Padded so the checksum byte ends a 16-byte block.
    at = (at + 1).div_ceil(16) * 16;
    if hash_appended {
        at += 32;
    }
    (at <= app.len()).then_some(at)
}

/// Send `app` (from [`app_image`]) to the board behind `call` and have it
/// boot it. `call` is a link's or the hub's request; `version` is only for
/// the board's log. `Err` says what went wrong, in words.
pub fn update(
    call: &dyn Fn(&str, Value, Duration) -> Result<Value, CallError>,
    app: &[u8],
    version: &str,
    mut progress: impl FnMut(Progress),
) -> Result<(), String> {
    let sha: String = Sha256::digest(app).iter().map(|b| format!("{b:02x}")).collect();
    let begun = call("ota/begin", json!({"size": app.len(), "sha256": sha, "version": version}), BEGIN_TIMEOUT).map_err(|e| match e {
        CallError::Rpc { code: -32601, .. } => "this board's firmware can't update over WiFi: flash it over USB once".to_string(),
        other => format!("starting the update: {other}"),
    })?;
    let chunk = begun["chunk"].as_u64().map_or(CHUNK, |c| c as usize).clamp(256, CHUNK);
    let b64 = base64::engine::general_purpose::STANDARD;
    let mut offset = 0;
    progress(Progress::Sent(0.0));
    while offset < app.len() {
        let end = (offset + chunk).min(app.len());
        let params = json!({"offset": offset, "data": b64.encode(&app[offset..end])});
        let mut tries = 0;
        loop {
            match call("ota/write", params.clone(), WRITE_TIMEOUT) {
                Ok(reply) if reply["written"].as_u64() == Some(end as u64) => break,
                Ok(reply) => return Err(fail(call, format!("the board wrote {} bytes, not {end}", reply["written"]))),
                // A reply lost on the way: where the board stands decides.
                Err(CallError::Timeout) if tries < RETRIES => {
                    tries += 1;
                    let written = call("ota/status", json!({}), WRITE_TIMEOUT).ok().and_then(|s| s["written"].as_u64());
                    if written == Some(end as u64) {
                        break;
                    }
                }
                Err(e) => return Err(fail(call, format!("sending the update: {e}"))),
            }
        }
        offset = end;
        progress(Progress::Sent(offset as f32 / app.len() as f32));
    }
    call("ota/end", json!({}), END_TIMEOUT).map_err(|e| fail(call, format!("finishing the update: {e}")))?;
    progress(Progress::Restarting);
    Ok(())
}

/// Leave the board clean after `why`, and hand `why` back.
fn fail(call: &dyn Fn(&str, Value, Duration) -> Result<Value, CallError>, why: String) -> String {
    let _ = call("ota/abort", json!({}), WRITE_TIMEOUT);
    why
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A tiny image: header, one 5-byte segment, padding, checksum, hash.
    fn image(hash: bool) -> Vec<u8> {
        let mut v = vec![0u8; 24];
        v[0] = 0xE9;
        v[1] = 1;
        v[23] = hash as u8;
        v.extend_from_slice(&0x3c00_0000u32.to_le_bytes());
        v.extend_from_slice(&5u32.to_le_bytes());
        v.extend_from_slice(&[1, 2, 3, 4, 5]);
        // 37 bytes so far: padding to 47, the checksum at 47.
        v.resize(48, 0);
        if hash {
            v.extend_from_slice(&[0xAB; 32]);
        }
        v
    }

    #[test]
    fn image_ends_after_checksum_and_hash() {
        assert_eq!(image_len(&image(false)), Some(48));
        assert_eq!(image_len(&image(true)), Some(80));
        let mut padded = image(true);
        padded.extend_from_slice(&[0xFF; 100]);
        assert_eq!(image_len(&padded), Some(80));
        assert_eq!(image_len(&[0u8; 64]), None);
    }

    #[test]
    fn sends_in_order_and_ends() {
        use std::cell::RefCell;
        let app = vec![7u8; 5000];
        let written = RefCell::new(0u64);
        let methods = RefCell::new(Vec::new());
        let call = |m: &str, p: Value, _: Duration| -> Result<Value, CallError> {
            methods.borrow_mut().push(m.to_string());
            match m {
                "ota/begin" => Ok(json!({"chunk": 2048})),
                "ota/write" => {
                    assert_eq!(p["offset"].as_u64(), Some(*written.borrow()));
                    let n = base64::engine::general_purpose::STANDARD.decode(p["data"].as_str().unwrap()).unwrap().len();
                    *written.borrow_mut() += n as u64;
                    Ok(json!({"written": *written.borrow()}))
                }
                _ => Ok(json!({})),
            }
        };
        let mut last = None;
        update(&call, &app, "0.19.0", |p| last = Some(p)).unwrap();
        assert_eq!(*written.borrow(), 5000);
        assert_eq!(methods.borrow().iter().filter(|m| *m == "ota/write").count(), 3);
        assert_eq!(methods.borrow().last().map(String::as_str), Some("ota/end"));
        assert_eq!(last, Some(Progress::Restarting));
    }
}
