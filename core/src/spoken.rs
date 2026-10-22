//! What Yoru said with ElevenLabs: the text sent and the sound that came
//! back, one clip per sentence, kept in `spoken/` in the data folder so the
//! app can list them and play them again. Each clip is `<id>.wav` with
//! `<id>.json` beside it; the oldest go once there are more than [`KEEP`].

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::voice::SAMPLE_RATE;

/// About 160 KB a sentence: a few hundred are tens of MB at most.
const KEEP: usize = 500;

/// A sentence ElevenLabs spoke.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Clip {
    /// When it was spoken (`20261022-153012-123`): its files' name, and its order.
    pub id: String,
    /// Unix milliseconds.
    pub at: i64,
    /// The text as sent, audio tags (`[laughs]`) and all.
    pub text: String,
    pub language: String,
    /// `elevenlabs:voice/<voice_id>`.
    pub voice: String,
    pub duration_ms: u64,
}

fn dir() -> io::Result<PathBuf> {
    crate::paths::data_dir().map(|d| d.join("spoken")).ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no data folder"))
}

/// Keep `samples` (16 kHz) as the sound of `text`.
pub fn save(text: &str, language: &str, voice: &str, samples: &[i16]) -> io::Result<Clip> {
    save_in(&dir()?, text, language, voice, samples, chrono::Local::now())
}

fn save_in(dir: &Path, text: &str, language: &str, voice: &str, samples: &[i16], now: chrono::DateTime<chrono::Local>) -> io::Result<Clip> {
    fs::create_dir_all(dir)?;
    let mut id = now.format("%Y%m%d-%H%M%S-%3f").to_string();
    // Two sentences in the same millisecond: the second gets a suffix.
    let base = id.clone();
    for n in 1.. {
        if !dir.join(format!("{id}.json")).exists() {
            break;
        }
        id = format!("{base}-{n}");
    }
    let clip = Clip {
        id,
        at: now.timestamp_millis(),
        text: text.to_string(),
        language: language.to_string(),
        voice: voice.to_string(),
        duration_ms: samples.len() as u64 * 1000 / SAMPLE_RATE as u64,
    };
    fs::write(dir.join(format!("{}.wav", clip.id)), crate::voice::wav(samples, SAMPLE_RATE))?;
    // The JSON last: a clip is listed only once its sound is there.
    fs::write(dir.join(format!("{}.json", clip.id)), serde_json::to_vec_pretty(&clip)?)?;
    prune(dir, KEEP);
    Ok(clip)
}

/// Every clip, newest first.
pub fn list() -> Vec<Clip> {
    dir().map(|d| list_in(&d)).unwrap_or_default()
}

fn list_in(dir: &Path) -> Vec<Clip> {
    let Ok(entries) = fs::read_dir(dir) else { return Vec::new() };
    let mut clips: Vec<Clip> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .filter_map(|p| serde_json::from_slice(&fs::read(p).ok()?).ok())
        .collect();
    clips.sort_by(|a, b| b.id.cmp(&a.id));
    clips
}

/// The WAV of clip `id`.
pub fn audio(id: &str) -> io::Result<Vec<u8>> {
    fs::read(dir()?.join(format!("{}.wav", checked(id)?)))
}

pub fn delete(id: &str) -> io::Result<()> {
    remove(&dir()?, checked(id)?);
    Ok(())
}

/// Forget every clip.
pub fn clear() -> io::Result<()> {
    let dir = dir()?;
    for clip in list_in(&dir) {
        remove(&dir, &clip.id);
    }
    Ok(())
}

fn remove(dir: &Path, id: &str) {
    let _ = fs::remove_file(dir.join(format!("{id}.json")));
    let _ = fs::remove_file(dir.join(format!("{id}.wav")));
}

fn prune(dir: &Path, keep: usize) {
    for clip in list_in(dir).into_iter().skip(keep) {
        remove(dir, &clip.id);
    }
}

/// An id from the app, made sure it names a file in the folder and nothing else.
fn checked(id: &str) -> io::Result<&str> {
    if !id.is_empty() && id.chars().all(|c| c.is_ascii_digit() || c == '-') {
        Ok(id)
    } else {
        Err(io::Error::new(io::ErrorKind::InvalidInput, format!("no clip {id}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn keeps_lists_and_prunes() {
        let dir = std::env::temp_dir().join(format!("yoru-spoken-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let t = chrono::Local.with_ymd_and_hms(2026, 10, 22, 15, 30, 12).unwrap();
        let a = save_in(&dir, "[laughs] Ciao", "it", "elevenlabs:voice/x", &[0; 16_000], t).unwrap();
        let b = save_in(&dir, "Again", "en", "elevenlabs:voice/x", &[0; 8_000], t).unwrap();
        assert_eq!((a.id.as_str(), a.duration_ms), ("20261022-153012-000", 1000));
        assert_eq!(b.id, "20261022-153012-000-1");
        assert_eq!(list_in(&dir).iter().map(|c| c.text.as_str()).collect::<Vec<_>>(), ["Again", "[laughs] Ciao"]);
        prune(&dir, 1);
        assert_eq!(list_in(&dir), [b]);
        assert!(!dir.join(format!("{}.wav", a.id)).exists());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn refuses_paths() {
        assert!(checked("20261022-153012-000").is_ok());
        assert!(checked("../keys").is_err() && checked("").is_err());
    }
}
