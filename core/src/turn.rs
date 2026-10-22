//! Whether the user finished the sentence: Pipecat's Smart Turn v3.2, a
//! Whisper Tiny encoder with a small head, run with tract (pure Rust) on the
//! last 8 s of the utterance. It hears intonation and rhythm, not words: a
//! pause to think sounds unfinished, the end of a question finished.
//!
//! The board tells the host each pause (`utterance_pause`) and, while this
//! is on, waits longer before ending the utterance itself (`voice/pause`);
//! when the model says the sentence is finished the host stops it
//! (`voice/stop`) at once. See [`crate::voice::Turns`].

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use rustfft::FftPlanner;
use rustfft::num_complex::Complex;
use tract_onnx::prelude::*;

const RATE: usize = 16_000;
/// The model hears the last this many seconds, zeros before when shorter.
const SECONDS: usize = 8;
const N_FFT: usize = 400;
const HOP: usize = 160;
const BINS: usize = N_FFT / 2 + 1;
const MELS: usize = 80;
const FRAMES: usize = SECONDS * RATE / HOP;
/// Above this the sentence is finished (Pipecat's own line).
pub const FINISHED: f32 = 0.5;

type Runnable = std::sync::Arc<TypedRunnableModel>;

/// The model, loaded on first use. Shared between connections.
pub struct SmartTurn {
    path: PathBuf,
    model: Mutex<Option<Runnable>>,
}

impl std::fmt::Debug for SmartTurn {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SmartTurn").field("path", &self.path).finish_non_exhaustive()
    }
}

impl SmartTurn {
    pub fn new(path: PathBuf) -> Self {
        Self { path, model: Mutex::new(None) }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Load it now, so the first pause doesn't wait.
    pub fn warm_up(&self) -> Result<(), String> {
        self.with(|_| Ok(()))
    }

    /// How sure it is that `samples` (16 kHz) end a finished sentence, 0..1.
    pub fn finished(&self, samples: &[i16]) -> Result<f32, String> {
        let audio: Vec<f32> = samples.iter().map(|&s| s as f32 / 32768.0).collect();
        let input: Tensor = tract_ndarray::Array3::from_shape_vec((1, MELS, FRAMES), features(&audio)).map_err(|e| e.to_string())?.into();
        self.with(|model| {
            let out = model.run(tvec!(input.into())).map_err(|e| e.to_string())?;
            let view = out[0].to_plain_array_view::<f32>().map_err(|e| e.to_string())?;
            view.iter().next().copied().ok_or_else(|| "no output".into())
        })
    }

    fn with<T>(&self, f: impl FnOnce(&Runnable) -> Result<T, String>) -> Result<T, String> {
        let mut model = self.model.lock().unwrap();
        if model.is_none() {
            *model = Some(load(&self.path)?);
        }
        f(model.as_ref().unwrap())
    }
}

fn load(path: &Path) -> Result<Runnable, String> {
    if !path.is_file() {
        return Err(format!("no model at {}", path.display()));
    }
    let model = tract_onnx::onnx().model_for_path(path).and_then(|m| m.into_optimized()).and_then(|m| m.into_runnable());
    model.map_err(|e| format!("Smart Turn: {e}"))
}

/// Whisper's log-mel features of the last [`SECONDS`] of `audio`, as
/// `WhisperFeatureExtractor(chunk_length=8)` makes them with
/// `do_normalize=True`: [`MELS`] rows of [`FRAMES`].
fn features(audio: &[f32]) -> Vec<f32> {
    let n = SECONDS * RATE;
    let mut x = vec![0f32; n];
    let take = audio.len().min(n);
    x[n - take..].copy_from_slice(&audio[audio.len() - take..]);
    // Zero mean, unit variance, over the padding too.
    let mean = x.iter().map(|&v| v as f64).sum::<f64>() / n as f64;
    let var = x.iter().map(|&v| (v as f64 - mean).powi(2)).sum::<f64>() / n as f64;
    let scale = 1.0 / (var + 1e-7).sqrt();
    x.iter_mut().for_each(|v| *v = ((*v as f64 - mean) * scale) as f32);
    // Frames centred on each hop, the signal reflected at the ends.
    let at = |i: isize| {
        let last = n as isize - 1;
        x[(if i < 0 { -i } else if i > last { 2 * last - i } else { i }) as usize]
    };
    let window: Vec<f32> = (0..N_FFT).map(|i| 0.5 - 0.5 * (2.0 * std::f32::consts::PI * i as f32 / N_FFT as f32).cos()).collect();
    let fft = FftPlanner::<f32>::new().plan_fft_forward(N_FFT);
    let bank = mel_filters();
    let mut out = vec![0f32; MELS * FRAMES];
    let mut buf = vec![Complex::new(0f32, 0f32); N_FFT];
    let mut power = [0f32; BINS];
    for t in 0..FRAMES {
        let start = (t * HOP) as isize - (N_FFT / 2) as isize;
        for (i, b) in buf.iter_mut().enumerate() {
            *b = Complex::new(at(start + i as isize) * window[i], 0.0);
        }
        fft.process(&mut buf);
        for (p, b) in power.iter_mut().zip(&buf) {
            *p = b.norm_sqr();
        }
        for (m, row) in bank.iter().enumerate() {
            let e: f32 = row.iter().zip(&power).map(|(w, p)| w * p).sum();
            out[m * FRAMES + t] = e.max(1e-10).log10();
        }
    }
    let floor = out.iter().copied().fold(f32::MIN, f32::max) - 8.0;
    out.iter_mut().for_each(|v| *v = (v.max(floor) + 4.0) / 4.0);
    out
}

/// Slaney's mel scale: linear below 1 kHz, logarithmic above.
fn hz_to_mel(f: f64) -> f64 {
    if f < 1000.0 { 3.0 * f / 200.0 } else { 15.0 + (f / 1000.0).ln() * 27.0 / 6.4f64.ln() }
}

fn mel_to_hz(m: f64) -> f64 {
    if m < 15.0 { 200.0 * m / 3.0 } else { 1000.0 * ((m - 15.0) * 6.4f64.ln() / 27.0).exp() }
}

/// Triangular filters from 0 to 8 kHz, each of unit area (Slaney's norm).
fn mel_filters() -> Vec<[f32; BINS]> {
    let (lo, hi) = (hz_to_mel(0.0), hz_to_mel(RATE as f64 / 2.0));
    let edges: Vec<f64> = (0..MELS + 2).map(|i| mel_to_hz(lo + (hi - lo) * i as f64 / (MELS + 1) as f64)).collect();
    (0..MELS)
        .map(|m| {
            let norm = 2.0 / (edges[m + 2] - edges[m]);
            let mut row = [0f32; BINS];
            for (k, w) in row.iter_mut().enumerate() {
                let f = k as f64 * RATE as f64 / N_FFT as f64;
                let rise = (f - edges[m]) / (edges[m + 1] - edges[m]);
                let fall = (edges[m + 2] - f) / (edges[m + 2] - edges[m + 1]);
                *w = (rise.min(fall).max(0.0) * norm) as f32;
            }
            row
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn features_have_whispers_shape_and_range() {
        // A second of a 440 Hz tone, the rest silence.
        let tone: Vec<f32> = (0..RATE).map(|i| (i as f32 * 440.0 * 2.0 * std::f32::consts::PI / RATE as f32).sin() * 0.3).collect();
        let f = features(&tone);
        assert_eq!(f.len(), MELS * FRAMES);
        let (lo, hi) = f.iter().fold((f32::MAX, f32::MIN), |(lo, hi), &v| (lo.min(v), hi.max(v)));
        // Clamped to 8 (log10) below the top, then (x + 4) / 4.
        assert!((hi - lo - 2.0).abs() < 1e-3, "{lo}..{hi}");
        // The tone is at the end, where the audio is kept.
        let mel_440 = (0..MELS).max_by(|&a, &b| f[a * FRAMES + FRAMES - 10].total_cmp(&f[b * FRAMES + FRAMES - 10])).unwrap();
        assert!((5..15).contains(&mel_440), "{mel_440}");
    }

    /// With the model at `YORU_SMART_TURN` and a 16 kHz mono WAV of a whole
    /// sentence at `YORU_TURN_WAV`: finished, and its first half not.
    #[test]
    #[ignore]
    fn the_model_hears_a_finished_sentence() {
        let model = SmartTurn::new(std::env::var("YORU_SMART_TURN").unwrap().into());
        let wav = std::fs::read(std::env::var("YORU_TURN_WAV").unwrap()).unwrap();
        let samples: Vec<i16> = wav[44..].as_chunks::<2>().0.iter().map(|c| i16::from_le_bytes(*c)).collect();
        let whole = model.finished(&samples).unwrap();
        let half = model.finished(&samples[..samples.len() * 55 / 100]).unwrap();
        assert!(whole > FINISHED && half < FINISHED, "whole {whole}, half {half}");
    }

    #[test]
    fn filters_cover_the_band() {
        let bank = mel_filters();
        assert_eq!(bank.len(), MELS);
        assert!(bank.iter().all(|row| row.iter().any(|&w| w > 0.0)));
    }
}
