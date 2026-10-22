//! What Yoru said with ElevenLabs: the text sent and the sound that came
//! back, one clip per sentence, kept in `spoken/` in the data folder so the
//! app can list them, play them again and export them. Each clip is
//! `<id>.wav` with `<id>.json` beside it; they stay until the user deletes them.

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::voice::SAMPLE_RATE;

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

/// Put clips `ids` (every clip when empty) in a zip in Downloads: each WAV
/// with its JSON, and `metadata.csv` (`file|text|language`, the LJSpeech
/// layout TTS tools read). Returns the zip's path.
pub fn export(ids: &[String]) -> io::Result<PathBuf> {
    let dest = crate::paths::home_dir().map(|h| h.join("Downloads")).ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no home folder"))?;
    export_in(&dir()?, ids, &dest, chrono::Local::now())
}

fn export_in(dir: &Path, ids: &[String], dest: &Path, now: chrono::DateTime<chrono::Local>) -> io::Result<PathBuf> {
    let mut clips = list_in(dir);
    if !ids.is_empty() {
        clips.retain(|c| ids.contains(&c.id));
    }
    if clips.is_empty() {
        return Err(io::Error::new(io::ErrorKind::NotFound, "no clips to export"));
    }
    clips.reverse(); // oldest first, as they were said
    fs::create_dir_all(dest)?;
    let base = format!("yoru-spoken-{}", now.format("%Y%m%d-%H%M%S"));
    let mut path = dest.join(format!("{base}.zip"));
    for n in 1.. {
        if !path.exists() {
            break;
        }
        path = dest.join(format!("{base}-{n}.zip"));
    }
    // Written aside and renamed, so a half-written zip never shows up.
    let part = path.with_extension("zip.part");
    let mut zip = Zip::new(io::BufWriter::new(fs::File::create(&part)?), now);
    let mut csv = String::from("file|text|language\n");
    for c in &clips {
        zip.add(&format!("{base}/wavs/{}.wav", c.id), &fs::read(dir.join(format!("{}.wav", c.id)))?)?;
        zip.add(&format!("{base}/clips/{}.json", c.id), &serde_json::to_vec_pretty(c)?)?;
        csv.push_str(&format!("{}|{}|{}\n", c.id, c.text.replace(['|', '\n', '\r'], " "), c.language));
    }
    zip.add(&format!("{base}/metadata.csv"), csv.as_bytes())?;
    zip.finish()?.into_inner().map_err(|e| e.into_error())?.sync_all()?;
    fs::rename(&part, &path)?;
    Ok(path)
}

/// A zip without compression: WAVs barely shrink, and it needs no crate
/// beyond `crc32fast`. Fine below 4 GB and 65,535 files.
struct Zip<W: Write> {
    out: W,
    offset: u32,
    central: Vec<u8>,
    entries: u16,
    /// MS-DOS time and date, the only kind a plain zip entry has.
    time: u16,
    date: u16,
}

impl<W: Write> Zip<W> {
    fn new(out: W, now: chrono::DateTime<chrono::Local>) -> Self {
        use chrono::{Datelike, Timelike};
        let time = (now.hour() << 11 | now.minute() << 5 | now.second() / 2) as u16;
        let date = (((now.year() - 1980).max(0) as u32) << 9 | now.month() << 5 | now.day()) as u16;
        Self { out, offset: 0, central: Vec::new(), entries: 0, time, date }
    }

    fn add(&mut self, name: &str, data: &[u8]) -> io::Result<()> {
        let too_big = || io::Error::other("export too big for a zip");
        let size = u32::try_from(data.len()).map_err(|_| too_big())?;
        let crc = crc32fast::hash(data);
        // The fields both headers share, from "version needed" to the name's length.
        let mut common = Vec::with_capacity(26);
        common.extend_from_slice(&20u16.to_le_bytes()); // version needed: 2.0
        common.extend_from_slice(&0x0800u16.to_le_bytes()); // UTF-8 names
        common.extend_from_slice(&0u16.to_le_bytes()); // stored
        common.extend_from_slice(&self.time.to_le_bytes());
        common.extend_from_slice(&self.date.to_le_bytes());
        common.extend_from_slice(&crc.to_le_bytes());
        common.extend_from_slice(&size.to_le_bytes());
        common.extend_from_slice(&size.to_le_bytes());
        common.extend_from_slice(&(name.len() as u16).to_le_bytes());

        self.out.write_all(&0x04034b50u32.to_le_bytes())?;
        self.out.write_all(&common)?;
        self.out.write_all(&0u16.to_le_bytes())?; // no extra field
        self.out.write_all(name.as_bytes())?;
        self.out.write_all(data)?;

        self.central.extend_from_slice(&0x02014b50u32.to_le_bytes());
        self.central.extend_from_slice(&20u16.to_le_bytes()); // made by
        self.central.extend_from_slice(&common);
        self.central.extend_from_slice(&[0; 12]); // extra, comment, disk, attributes
        self.central.extend_from_slice(&self.offset.to_le_bytes());
        self.central.extend_from_slice(name.as_bytes());

        let written = 30 + name.len() as u32 + size;
        self.offset = self.offset.checked_add(written).ok_or_else(too_big)?;
        self.entries = self.entries.checked_add(1).ok_or_else(too_big)?;
        Ok(())
    }

    fn finish(mut self) -> io::Result<W> {
        self.out.write_all(&self.central)?;
        self.out.write_all(&0x06054b50u32.to_le_bytes())?;
        self.out.write_all(&[0; 4])?; // this disk, the central directory's disk
        self.out.write_all(&self.entries.to_le_bytes())?;
        self.out.write_all(&self.entries.to_le_bytes())?;
        self.out.write_all(&(self.central.len() as u32).to_le_bytes())?;
        self.out.write_all(&self.offset.to_le_bytes())?;
        self.out.write_all(&0u16.to_le_bytes())?; // no comment
        self.out.flush()?;
        Ok(self.out)
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
    fn keeps_and_lists() {
        let dir = std::env::temp_dir().join(format!("yoru-spoken-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let t = chrono::Local.with_ymd_and_hms(2026, 10, 22, 15, 30, 12).unwrap();
        let a = save_in(&dir, "[laughs] Ciao", "it", "elevenlabs:voice/x", &[0; 16_000], t).unwrap();
        let b = save_in(&dir, "Again", "en", "elevenlabs:voice/x", &[0; 8_000], t).unwrap();
        assert_eq!((a.id.as_str(), a.duration_ms), ("20261022-153012-000", 1000));
        assert_eq!(b.id, "20261022-153012-000-1");
        assert_eq!(list_in(&dir).iter().map(|c| c.text.as_str()).collect::<Vec<_>>(), ["Again", "[laughs] Ciao"]);
        remove(&dir, &a.id);
        assert_eq!(list_in(&dir), [b]);
        assert!(!dir.join(format!("{}.wav", a.id)).exists());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn exports_a_zip_unzip_reads() {
        let dir = std::env::temp_dir().join(format!("yoru-spoken-zip-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let t = chrono::Local.with_ymd_and_hms(2026, 10, 22, 15, 30, 12).unwrap();
        let a = save_in(&dir, "Ciao | tu", "it", "elevenlabs:voice/x", &[1; 1_600], t).unwrap();
        save_in(&dir, "Left out", "en", "elevenlabs:voice/x", &[2; 1_600], t).unwrap();
        let out = dir.join("out");
        let zip = export_in(&dir, std::slice::from_ref(&a.id), &out, t).unwrap();
        assert_eq!(zip.file_name().unwrap(), "yoru-spoken-20261022-153012.zip");
        assert!(export_in(&dir, &["nope".into()], &out, t).is_err());
        // The system's unzip as the judge, where there is one.
        if let Ok(o) = std::process::Command::new("unzip").arg("-p").arg(&zip).arg("yoru-spoken-20261022-153012/metadata.csv").output() {
            assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
            assert_eq!(String::from_utf8(o.stdout).unwrap(), format!("file|text|language\n{}|Ciao   tu|it\n", a.id));
            let t = std::process::Command::new("unzip").arg("-tq").arg(&zip).output().unwrap();
            assert!(t.status.success(), "{}", String::from_utf8_lossy(&t.stdout));
        }
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn refuses_paths() {
        assert!(checked("20261022-153012-000").is_ok());
        assert!(checked("../keys").is_err() && checked("").is_err());
    }
}
