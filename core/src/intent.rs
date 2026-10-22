//! What a follow-up means: whether the words heard after an answer, without
//! the wake word, close the conversation ("niente, grazie"), only
//! acknowledge it ("ok"), or go on with it. [`IntentModel`] compares them
//! with a few dozen examples of each, embedded by Google's EmbeddingGemma on
//! a llama.cpp `llama-server --embedding` sidecar ([`crate::sidecar`]), and
//! takes the nearest ones' vote. It only says "close" or "ack" when it is
//! sure: anything else goes on to the assistant, which can tell.

use std::io;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Mutex;
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::sidecar::{self, Process};
use crate::stt::models_dir;

/// Loading 300 MB takes a second or two.
const STARTUP_TIMEOUT: Duration = Duration::from_secs(60);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
/// EmbeddingGemma's prompt for classification, before every text.
const PROMPT: &str = "task: classification | query: ";
/// The nearest examples that vote.
const VOTERS: usize = 3;
/// Closing or acknowledging needs an example at least this close (cosine):
/// "mettilo alle sei" came out nearest to "a posto così" at 0.70.
const SURE: f32 = 0.80;

/// What a follow-up does to the conversation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Meaning {
    /// It's over: "grazie", "niente", "basta così".
    Close,
    /// "Ok", "va bene": over after a plain answer, a yes after a question.
    Ack,
    /// Anything else: the assistant answers it.
    Other,
}

/// The examples, in Italian and English. Nothing here has to match: the
/// nearest ones in meaning vote.
const EXAMPLES: &[(Meaning, &[&str])] = &[
    (
        Meaning::Close,
        &[
            "grazie", "grazie mille", "no grazie", "niente", "nient'altro", "nulla", "basta così", "è tutto", "a posto così",
            "lascia stare", "lascia perdere", "non importa", "non serve", "ok grazie, a dopo", "va bene così, grazie", "per ora è tutto",
            "niente, grazie", "capito, grazie", "ci penso dopo", "ciao", "a dopo", "thanks", "thank you", "no thanks", "that's all",
            "nothing", "never mind", "that's it for now", "bye", "all good, thanks", "no, that's all",
        ],
    ),
    (
        Meaning::Ack,
        &[
            "ok", "okay", "va bene", "perfetto", "d'accordo", "capito", "bene", "ottimo", "fantastico", "ah ok", "got it", "alright",
            "sounds good", "great", "perfect", "cool",
        ],
    ),
    (
        Meaning::Other,
        &[
            "che tempo fa domani?", "metti un timer di dieci minuti", "sì", "sì grazie, mandala", "no, alle cinque", "leggimi le mail",
            "e domani?", "chi ha vinto la partita?", "aggiungilo al calendario", "sì, per favore", "no, intendevo martedì",
            "ricordami di chiamare Marco", "quanto manca alla riunione?", "dimmi una barzelletta", "alza il volume",
            "grazie, e domani che tempo fa?", "sì, mandala pure", "no, cambia l'orario", "sì, grazie", "no", "no, non quella",
            "sì, va bene, fallo", "mettilo alle otto", "spostalo a domani", "quanto fa venti per tre", "niente di nuovo nelle mail?",
            "basta con questi avvisi", "togli le notifiche", "leggimi la prossima", "what's the weather tomorrow", "yes please",
            "set a timer for five minutes", "no, at five", "read my email", "and tomorrow?", "yes, thanks", "no, the other one",
            "move it to friday",
        ],
    ),
];

/// The sidecar and the examples' embeddings. Shared between connections.
pub struct IntentModel {
    server: PathBuf,
    model: PathBuf,
    running: Mutex<Option<Running>>,
}

struct Running {
    process: Process,
    examples: Vec<(Meaning, Vec<f32>)>,
}

impl std::fmt::Debug for IntentModel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IntentModel").field("model", &self.model).finish_non_exhaustive()
    }
}

impl IntentModel {
    /// With the model file at `model`; `None` without a `llama-server`.
    pub fn new(model: PathBuf) -> Option<Self> {
        Some(Self { server: crate::llm::find_server()?, model, running: Mutex::new(None) })
    }

    pub fn path(&self) -> &std::path::Path {
        &self.model
    }

    /// Start the server and embed the examples now, so the first follow-up doesn't wait.
    pub fn warm_up(&self) -> Result<(), String> {
        self.with(|_| Ok(()))
    }

    /// Stop the server; the next follow-up starts it again.
    pub fn shutdown(&self) {
        self.running.lock().unwrap().take();
    }

    /// What `text` means.
    pub fn classify(&self, text: &str) -> Result<Meaning, String> {
        self.with(|running| {
            let v = embed(running.process.addr, &[text])?.pop().ok_or("no embedding")?;
            Ok(vote(&running.examples, &v))
        })
    }

    fn with<T>(&self, f: impl FnOnce(&Running) -> Result<T, String>) -> Result<T, String> {
        let mut running = self.running.lock().unwrap();
        if !running.as_mut().is_some_and(|r| r.process.is_running()) {
            *running = Some(self.start()?);
        }
        f(running.as_ref().unwrap())
    }

    fn start(&self) -> Result<Running, String> {
        if !self.model.is_file() {
            return Err(format!("no model at {}", self.model.display()));
        }
        let mut cmd = Command::new(&self.server);
        cmd.arg("--model").arg(&self.model);
        cmd.args(["--embedding", "--pooling", "mean", "--ctx-size", "2048", "--ubatch-size", "2048", "--parallel", "1", "--no-webui"]);
        let dir = models_dir();
        let (pid_file, log) = (dir.as_ref().map(|d| d.join("llama-embed.pid")), dir.as_ref().map(|d| d.join("llama-embed.log")));
        let started = Instant::now();
        let process = Process::start(cmd, "llama-server", pid_file, log, STARTUP_TIMEOUT)?;
        // It listens at once, and answers 503 until the model is loaded.
        while sidecar::get(process.addr, "/health", Duration::from_secs(5)).is_err() {
            if started.elapsed() > STARTUP_TIMEOUT {
                return Err("the embedding model didn't load in time (see llama-embed.log)".into());
            }
            thread::sleep(Duration::from_millis(200));
        }
        let texts: Vec<&str> = EXAMPLES.iter().flat_map(|(_, t)| t.iter().copied()).collect();
        let vectors = embed(process.addr, &texts)?;
        let meanings = EXAMPLES.iter().flat_map(|(m, t)| t.iter().map(move |_| *m));
        Ok(Running { process, examples: meanings.zip(vectors).collect() })
    }
}

/// Unit vectors for `texts`, in order.
fn embed(addr: std::net::SocketAddr, texts: &[&str]) -> Result<Vec<Vec<f32>>, String> {
    let input: Vec<String> = texts.iter().map(|t| format!("{PROMPT}{t}")).collect();
    let body = serde_json::to_vec(&json!({ "input": input })).map_err(|e| e.to_string())?;
    let reply = sidecar::post(addr, "/v1/embeddings", "application/json", &body, REQUEST_TIMEOUT).map_err(|e: io::Error| e.to_string())?;
    let reply: Value = serde_json::from_slice(&reply).map_err(|e| e.to_string())?;
    let data = reply["data"].as_array().ok_or_else(|| format!("no embeddings in {}", reply.to_string().chars().take(200).collect::<String>()))?;
    let mut out = vec![Vec::new(); texts.len()];
    for d in data {
        let i = d["index"].as_u64().unwrap_or(0) as usize;
        let v: Vec<f32> = d["embedding"].as_array().into_iter().flatten().filter_map(|x| x.as_f64()).map(|x| x as f32).collect();
        if let Some(slot) = out.get_mut(i) {
            *slot = unit(v);
        }
    }
    Ok(out)
}

fn unit(mut v: Vec<f32>) -> Vec<f32> {
    let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        v.iter_mut().for_each(|x| *x /= norm);
    }
    v
}

/// The nearest examples' vote, weighted by closeness; Close or Ack only
/// when the nearest one agrees and is near enough.
fn vote(examples: &[(Meaning, Vec<f32>)], v: &[f32]) -> Meaning {
    let mut near: Vec<(f32, Meaning)> = examples.iter().map(|(m, e)| (e.iter().zip(v).map(|(a, b)| a * b).sum(), *m)).collect();
    near.sort_by(|a, b| b.0.total_cmp(&a.0));
    let Some(&(best, nearest)) = near.first() else { return Meaning::Other };
    let score = |m: Meaning| near.iter().take(VOTERS).filter(|(_, n)| *n == m).map(|(s, _)| s).sum::<f32>();
    let won = [Meaning::Close, Meaning::Ack, Meaning::Other].into_iter().max_by(|a, b| score(*a).total_cmp(&score(*b))).unwrap_or(Meaning::Other);
    if won != Meaning::Other && (best < SURE || nearest != won) { Meaning::Other } else { won }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ex(m: Meaning, v: [f32; 2]) -> (Meaning, Vec<f32>) {
        (m, unit(v.to_vec()))
    }

    #[test]
    fn closes_only_when_sure() {
        let examples = vec![ex(Meaning::Close, [1.0, 0.0]), ex(Meaning::Close, [0.95, 0.05]), ex(Meaning::Other, [0.0, 1.0])];
        assert_eq!(vote(&examples, &unit(vec![1.0, 0.02])), Meaning::Close);
        // Nearest to a closing example, but not near enough.
        assert_eq!(vote(&examples, &unit(vec![0.5, 0.5])), Meaning::Other);
        assert_eq!(vote(&examples, &unit(vec![0.1, 1.0])), Meaning::Other);
    }

    /// With EmbeddingGemma at `YORU_EMBEDDING_MODEL` and a `llama-server`.
    #[test]
    #[ignore]
    fn the_model_tells_a_goodbye() {
        let model = IntentModel::new(std::env::var("YORU_EMBEDDING_MODEL").unwrap().into()).unwrap();
        for (text, want) in [
            ("Niente.", Meaning::Close),
            ("no, a posto così", Meaning::Close),
            ("ok grazie", Meaning::Close),
            ("thanks a lot", Meaning::Close),
            ("va bene", Meaning::Ack),
            ("sì", Meaning::Other),
            ("e dopodomani?", Meaning::Other),
            ("mettilo alle sei", Meaning::Other),
            ("basta con le notifiche di Amazon", Meaning::Other),
        ] {
            assert_eq!(model.classify(text).unwrap(), want, "{text}");
        }
    }

    #[test]
    fn every_meaning_has_examples() {
        for m in [Meaning::Close, Meaning::Ack, Meaning::Other] {
            assert!(EXAMPLES.iter().any(|(e, t)| *e == m && !t.is_empty()));
        }
    }
}
