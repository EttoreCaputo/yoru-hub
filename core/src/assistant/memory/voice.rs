//! Who speaks (docs/memory.md, Knowing who speaks): a voice print for each
//! person, kept on their node: the mean of the WeSpeaker embeddings
//! (256 numbers, from FluidAudio through `yoru-audio voiceprint`, see
//! [`crate::voiceprint`]) of what they said. An utterance is theirs when its
//! print is near enough to it. Alongside, how they usually talk (loudness,
//! pitch, pace), for telling how they feel now ([`super::feelings`]).

use serde::{Deserialize, Serialize};

use super::{Memory, from_blob, search::cosine, to_blob};

/// Two prints of one person are usually above 0.7, of two people below 0.5
/// (WeSpeaker on 3 s or more); between, nobody is named.
pub const SAME_SPEAKER: f32 = 0.6;

/// How a person usually talks, learnt slowly from what they say.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq)]
pub struct Usual {
    /// Prints averaged into their voice print.
    pub prints: u32,
    /// Utterances the rest was learnt from.
    pub heard: u32,
    /// dBFS.
    pub loudness: f32,
    /// Hz.
    pub pitch: f32,
    /// Words a second.
    pub pace: f32,
}

pub fn unit(mut v: Vec<f32>) -> Vec<f32> {
    let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        v.iter_mut().for_each(|x| *x /= norm);
    }
    v
}

impl Memory {
    /// The person whose voice `print` is, and how sure (cosine).
    pub fn identify(&self, print: &[f32]) -> Option<(i64, f32)> {
        let prints: Vec<(i64, Vec<u8>)> = self.with_db(|db| {
            let Ok(mut stmt) = db.prepare("SELECT id, voiceprint FROM entities WHERE voiceprint IS NOT NULL") else { return Vec::new() };
            stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?))).map(|rows| rows.flatten().collect()).unwrap_or_default()
        });
        let print = unit(print.to_vec());
        prints.into_iter().map(|(id, b)| (id, cosine(&print, &from_blob(&b)))).filter(|(_, c)| *c >= SAME_SPEAKER).max_by(|a, b| a.1.total_cmp(&b.1))
    }

    /// `print` is `person`'s voice: it joins their voice print.
    pub fn enrol(&self, person: i64, print: &[f32]) {
        let mut usual: Usual = self.json(person, "voice");
        let old: Option<Vec<f32>> = self.with_db(|db| {
            db.query_row("SELECT voiceprint FROM entities WHERE id = ?1", [person], |r| r.get::<_, Option<Vec<u8>>>(0)).ok().flatten().map(|b| from_blob(&b))
        });
        let print = unit(print.to_vec());
        // A running mean, the newest counting as much as one in twenty at least.
        let n = usual.prints.min(19) as f32;
        let mean = match old {
            Some(old) if old.len() == print.len() => unit(old.iter().zip(&print).map(|(o, p)| (o * n + p) / (n + 1.0)).collect()),
            _ => print,
        };
        usual.prints += 1;
        self.with_db(|db| {
            let _ = db.execute("UPDATE entities SET voiceprint = ?1 WHERE id = ?2", rusqlite::params![to_blob(&mean), person]);
        });
        self.set_json(person, "voice", &usual);
    }

    /// Forget `person`'s voice.
    pub fn forget_voice(&self, person: i64) {
        self.with_db(|db| {
            let _ = db.execute("UPDATE entities SET voiceprint = NULL WHERE id = ?1", [person]);
        });
        let mut usual: Usual = self.json(person, "voice");
        usual.prints = 0;
        self.set_json(person, "voice", &usual);
    }

    /// Whether anyone's voice is known.
    pub fn knows_voices(&self) -> bool {
        self.with_db(|db| db.query_row("SELECT count(*) FROM entities WHERE voiceprint IS NOT NULL", [], |r| r.get::<_, i64>(0)).unwrap_or(0) > 0)
    }

    pub fn usual(&self, person: i64) -> Usual {
        self.json(person, "voice")
    }

    /// Learn a little more of how `person` usually talks.
    pub fn learn_usual(&self, person: i64, loudness: f32, pitch: Option<f32>, pace: Option<f32>) {
        let mut u = self.usual(person);
        // The first few set it, then it moves slowly.
        let k = 1.0 / (u.heard.min(29) as f32 + 1.0);
        let mix = |old: f32, new: f32| if old == 0.0 { new } else { old + (new - old) * k };
        u.loudness = mix(u.loudness, loudness);
        if let Some(p) = pitch {
            u.pitch = mix(u.pitch, p);
        }
        if let Some(p) = pace {
            u.pace = mix(u.pace, p);
        }
        u.heard += 1;
        self.set_json(person, "voice", &u);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_voice_is_told_from_another() {
        let memory = Memory::open(None);
        let ettore = memory.user();
        let sofia = memory.resolve("Sofia", Some("person")).unwrap();
        memory.enrol(ettore, &[1.0, 0.1, 0.0]);
        memory.enrol(sofia, &[0.0, 0.2, 1.0]);
        assert_eq!(memory.identify(&[0.9, 0.2, 0.1]).unwrap().0, ettore);
        assert_eq!(memory.identify(&[0.1, 0.1, 0.9]).unwrap().0, sofia);
        assert!(memory.identify(&[0.0, 1.0, 0.0]).is_none());
        memory.merge(ettore, sofia).unwrap();
        assert!(memory.knows_voices());
    }
}
