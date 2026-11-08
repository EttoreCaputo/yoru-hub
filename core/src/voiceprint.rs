//! Voice prints of what is said to Yoru (docs/memory.md, Knowing who
//! speaks), from the `yoru-audio voiceprint` helper (FluidAudio's WeSpeaker
//! embedding on the Neural Engine), kept running once started so a print
//! takes tens of milliseconds. The models (about 30 MB) are downloaded
//! after the first request said aloud.

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use serde_json::{Value, json};

/// Loading takes a second or two; the first download, longer.
const START_TIMEOUT: Duration = Duration::from_secs(180);

pub struct Voiceprinter {
    helper: PathBuf,
    folder: PathBuf,
    running: Mutex<Option<Running>>,
    preparing: AtomicBool,
    /// The models are being downloaded: no prints until they are here.
    downloading: AtomicBool,
}

struct Running {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl std::fmt::Debug for Voiceprinter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Voiceprinter").field("folder", &self.folder).finish_non_exhaustive()
    }
}

impl Voiceprinter {
    /// With the helper installed with the app; `None` without it (not a Mac).
    pub fn new() -> Option<Self> {
        let helper = crate::sidecar::find_program("yoru-audio", "YORU_AUDIO")?;
        let folder = crate::stt::models_dir()?.join("voiceprint");
        Some(Self { helper, folder, running: Mutex::new(None), preparing: AtomicBool::new(false), downloading: AtomicBool::new(false) })
    }

    /// Whether the models are here (prints can be made without a download).
    pub fn is_ready(&self) -> bool {
        !self.downloading.load(Ordering::Relaxed) && std::fs::read_dir(self.folder.join("diarizer")).is_ok_and(|mut d| d.next().is_some())
    }

    /// Download the models and start the helper, on a thread of its own,
    /// once: prints are made after it.
    pub fn prepare(self: &std::sync::Arc<Self>) {
        if self.preparing.swap(true, Ordering::Relaxed) {
            return;
        }
        let me = self.clone();
        self.downloading.store(true, Ordering::Relaxed);
        std::thread::spawn(move || {
            let mut running = me.running.lock().unwrap();
            let started = me.start();
            me.downloading.store(false, Ordering::Relaxed);
            match started {
                Ok(r) => *running = Some(r),
                Err(e) => {
                    eprintln!("voice print: {e}");
                    me.preparing.store(false, Ordering::Relaxed);
                }
            }
        });
    }

    /// The print of `samples` (16 kHz mono).
    pub fn print(&self, samples: &[i16]) -> Result<Vec<f32>, String> {
        let tmp = std::env::temp_dir().join(format!("yoru-voice-{}-{:?}.wav", std::process::id(), std::thread::current().id()));
        std::fs::write(&tmp, crate::voice::wav(samples, 16_000)).map_err(|e| e.to_string())?;
        let out = self.ask(json!({"file": tmp}));
        let _ = std::fs::remove_file(&tmp);
        out
    }

    /// The print of the parts `ranges` (ms) of the audio file `file`: one
    /// speaker's lines in a meeting.
    pub fn print_file(&self, file: &Path, ranges: &[(u64, u64)]) -> Result<Vec<f32>, String> {
        self.ask(json!({"file": file, "ranges": ranges.iter().map(|(a, b)| [*a, *b]).collect::<Vec<_>>()}))
    }

    fn ask(&self, request: Value) -> Result<Vec<f32>, String> {
        let mut running = self.running.lock().unwrap();
        if running.as_mut().is_none_or(|r| r.child.try_wait().ok().flatten().is_some()) {
            *running = Some(self.start()?);
        }
        let r = running.as_mut().unwrap();
        let sent = writeln!(r.stdin, "{request}").and_then(|_| r.stdin.flush());
        if let Err(e) = sent {
            *running = None;
            return Err(e.to_string());
        }
        let mut line = String::new();
        if r.stdout.read_line(&mut line).map_err(|e| e.to_string())? == 0 {
            *running = None;
            return Err("the voice print helper stopped".into());
        }
        let answer: Value = serde_json::from_str(&line).map_err(|e| e.to_string())?;
        match answer["event"].as_str() {
            Some("print") => Ok(answer["embedding"].as_array().into_iter().flatten().filter_map(Value::as_f64).map(|x| x as f32).collect()),
            _ => Err(answer["message"].as_str().unwrap_or("no print").to_string()),
        }
    }

    fn start(&self) -> Result<Running, String> {
        let _ = std::fs::create_dir_all(&self.folder);
        let mut child = Command::new(&self.helper)
            .arg("voiceprint")
            .arg("--model")
            .arg(&self.folder)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("yoru-audio: {e}"))?;
        let stdin = child.stdin.take().ok_or("no stdin")?;
        let mut stdout = BufReader::new(child.stdout.take().ok_or("no stdout")?);
        // Wait for "ready" on a thread, so a stuck download times out.
        let (tx, rx) = std::sync::mpsc::channel();
        let reader = std::thread::spawn(move || {
            let mut line = String::new();
            let ok = loop {
                line.clear();
                match stdout.read_line(&mut line) {
                    Ok(0) | Err(_) => break Err("the voice print helper didn't start".to_string()),
                    Ok(_) => {
                        let v: Value = serde_json::from_str(&line).unwrap_or_default();
                        match v["event"].as_str() {
                            Some("ready") => break Ok(()),
                            Some("error") => break Err(v["message"].as_str().unwrap_or("error").to_string()),
                            _ => {}
                        }
                    }
                }
            };
            let _ = tx.send(ok);
            stdout
        });
        match rx.recv_timeout(START_TIMEOUT) {
            Ok(Ok(())) => {
                let stdout = reader.join().map_err(|_| "reader")?;
                Ok(Running { child, stdin, stdout })
            }
            Ok(Err(e)) => {
                let _ = child.kill();
                Err(e)
            }
            Err(_) => {
                let _ = child.kill();
                Err("the voice print models didn't load in time".into())
            }
        }
    }
}

/// Loudness (dBFS), pitch (Hz, median of the voiced frames) and length of
/// the voiced part (s) of 16 kHz speech: how the person said it.
pub fn prosody(samples: &[i16]) -> (f32, Option<f32>, f32) {
    const FRAME: usize = 640; // 40 ms
    let mut loud = Vec::new();
    let mut pitches = Vec::new();
    let mut voiced = 0usize;
    for frame in samples.chunks_exact(FRAME) {
        let x: Vec<f32> = frame.iter().map(|s| *s as f32 / 32768.0).collect();
        let rms = (x.iter().map(|v| v * v).sum::<f32>() / FRAME as f32).sqrt();
        let db = 20.0 * rms.max(1e-6).log10();
        if db < -45.0 {
            continue;
        }
        voiced += 1;
        loud.push(db);
        if let Some(p) = pitch(&x) {
            pitches.push(p);
        }
    }
    let loudness = if loud.is_empty() { -60.0 } else { loud.iter().sum::<f32>() / loud.len() as f32 };
    pitches.sort_by(f32::total_cmp);
    let median = (pitches.len() >= 3).then(|| pitches[pitches.len() / 2]);
    (loudness, median, voiced as f32 * FRAME as f32 / 16_000.0)
}

/// A frame's pitch by autocorrelation, 70 to 400 Hz, when it is clear.
fn pitch(x: &[f32]) -> Option<f32> {
    let energy: f32 = x.iter().map(|v| v * v).sum();
    if energy <= 0.0 {
        return None;
    }
    let (lo, hi) = (16_000 / 400, 16_000 / 70);
    let (lag, best) = (lo..=hi.min(x.len() - 1))
        .map(|lag| (lag, x.iter().zip(&x[lag..]).map(|(a, b)| a * b).sum::<f32>() / energy))
        .max_by(|a, b| a.1.total_cmp(&b.1))?;
    (best > 0.45).then(|| 16_000.0 / lag as f32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_a_tones_pitch_and_loudness() {
        let tone: Vec<i16> = (0..16_000).map(|i| ((i as f32 * 2.0 * std::f32::consts::PI * 200.0 / 16_000.0).sin() * 8000.0) as i16).collect();
        let (loud, pitch, voiced) = prosody(&tone);
        assert!((pitch.unwrap() - 200.0).abs() < 5.0, "{pitch:?}");
        assert!(loud > -16.0 && loud < -10.0, "{loud}");
        assert!((voiced - 1.0).abs() < 0.05);
    }
}
