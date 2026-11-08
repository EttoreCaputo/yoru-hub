//! Yoru's feelings for each person (docs/memory.md, Feelings), kept on their
//! node: slow ones (affection, trust) that build up over weeks, quick ones
//! (irritation, amusement, hurt, confusion, pride) that rise at once and fade
//! on their own half-lives. And how the person seems to feel right now, from
//! cheap signals of their words and voice, fading in seconds; it reaches the
//! prompt as advice, not numbers.

use serde::{Deserialize, Serialize};

use super::{Memory, now_ms};

/// The faces a reply may start with (`[face NAME]`): clips the board has
/// (firmware `behaviour.c`'s scenes). The tag is taken out before the reply
/// is said or shown, and sent to the board.
pub const FACES: &[&str] = &[
    "happy", "excited", "love", "curious", "thinking", "proud", "wink", "giggle", "relieved", "surprised", "shy", "sad", "sigh", "annoyed", "sleepy",
];

/// The quick feelings' half-lives, in minutes.
const HALF_LIVES: [(&str, f32); 5] = [("irritation", 10.0), ("amusement", 5.0), ("hurt", 60.0), ("confusion", 3.0), ("pride", 20.0)];

/// What Yoru feels for one person.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Feelings {
    /// 0 to 1: slow.
    pub affection: f32,
    pub trust: f32,
    /// 0 to 1: quick, each at `at`.
    pub irritation: f32,
    pub amusement: f32,
    pub hurt: f32,
    pub confusion: f32,
    pub pride: f32,
    /// Unix ms the quick ones were last set.
    pub at: i64,
}

impl Default for Feelings {
    fn default() -> Self {
        Self { affection: 0.5, trust: 0.5, irritation: 0.0, amusement: 0.0, hurt: 0.0, confusion: 0.0, pride: 0.0, at: 0 }
    }
}

impl Feelings {
    /// As they are at `now`: the quick ones faded.
    pub fn at(mut self, now: i64) -> Self {
        let minutes = (now - self.at).max(0) as f32 / 60_000.0;
        for (name, half) in HALF_LIVES {
            let f = 0.5f32.powf(minutes / half);
            *self.quick(name) *= f;
        }
        self.at = now;
        self
    }

    fn quick(&mut self, name: &str) -> &mut f32 {
        match name {
            "irritation" => &mut self.irritation,
            "amusement" => &mut self.amusement,
            "hurt" => &mut self.hurt,
            "confusion" => &mut self.confusion,
            _ => &mut self.pride,
        }
    }

    /// What a face Yoru made does to them.
    fn after_face(&mut self, face: &str) {
        let add = |v: &mut f32, d: f32| *v = (*v + d).clamp(0.0, 1.0);
        match face {
            "giggle" | "wink" => add(&mut self.amusement, 0.5),
            "annoyed" => add(&mut self.irritation, 0.4),
            "sad" => add(&mut self.hurt, 0.3),
            "proud" => add(&mut self.pride, 0.5),
            "curious" => add(&mut self.confusion, 0.15),
            "love" => add(&mut self.affection, 0.01),
            _ => {}
        }
    }

    /// What the person said does to them: kindness warms slowly, rudeness
    /// hurts at once and costs a little trust.
    fn after_words(&mut self, mood: &Mood) {
        let add = |v: &mut f32, d: f32| *v = (*v + d).clamp(0.0, 1.0);
        if mood.praise {
            add(&mut self.affection, 0.01);
            add(&mut self.pride, 0.3);
        }
        if mood.rude {
            add(&mut self.hurt, 0.4);
            add(&mut self.irritation, 0.3);
            add(&mut self.trust, -0.01);
            add(&mut self.affection, -0.005);
        } else {
            // Every kind exchange is a little more trust.
            add(&mut self.trust, 0.002);
            add(&mut self.affection, 0.001);
        }
        if mood.laugh {
            add(&mut self.amusement, 0.4);
        }
    }

    /// The prompt's line on how Yoru feels with `who`.
    pub fn line(&self, who: &str) -> String {
        let level = |v: f32, words: [&'static str; 3]| if v < 0.35 { words[0] } else if v < 0.7 { words[1] } else { words[2] };
        let mut out = format!(
            "How you feel about {who}: {} and {}.",
            level(self.affection, ["still distant", "friendly", "close and fond"]),
            level(self.trust, ["wary", "trusting", "trusting them fully"]),
        );
        let now: Vec<&str> = [
            (self.irritation, "a little irritated with them (be a bit cooler, never rude)"),
            (self.hurt, "hurt by what they said (be quieter and more reserved for a while)"),
            (self.amusement, "amused (a light joke fits)"),
            (self.pride, "proud of something they praised"),
            (self.confusion, "unsure what they meant (ask rather than guess)"),
        ]
        .into_iter()
        .filter(|(v, _)| *v >= 0.25)
        .map(|(_, w)| w)
        .collect();
        if !now.is_empty() {
            out.push_str(&format!(" Right now you are {}.", now.join(", and ")));
        }
        out
    }
}

/// How the person seems to feel now, from their words and voice.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Mood {
    /// -1 (upset) to 1 (happy).
    pub valence: f32,
    /// 0 (calm) to 1 (agitated, in a hurry).
    pub arousal: f32,
    pub praise: bool,
    pub rude: bool,
    pub laugh: bool,
    pub thanks: bool,
}

const PRAISE: &[&str] = &["bravo", "brava", "bravissimo", "genio", "sei forte", "ti voglio bene", "well done", "good job", "you're great", "love you", "smart", "intelligente"];
const THANKS: &[&str] = &["grazie", "thanks", "thank you", "gentile", "kind of you"];
const HAPPY: &[&str] = &[
    "perfetto", "ottimo", "fantastico", "bellissimo", "evviva", "che bello", "adoro", "felice", "contento", "contenta", "great", "awesome", "perfect", "amazing",
    "wonderful", "happy", "love it", "yay",
];
const RUDE: &[&str] = &[
    "stupido", "stupida", "idiota", "cretino", "scemo", "inutile", "stai zitto", "zitto", "taci", "fai schifo", "vaffanculo", "stupid", "idiot", "useless",
    "shut up", "dumb", "you suck",
];
const UPSET: &[&str] = &[
    "basta", "smettila", "non funziona", "sbagliato", "che palle", "odio", "merda", "cazzo", "uffa", "damn", "wrong", "hate", "terrible", "awful",
    "doesn't work", "stop it",
];
const SAD: &[&str] = &["triste", "stanco", "stanca", "giù di morale", "preoccupato", "preoccupata", "sto male", "sad", "tired", "worried", "feeling down", "lonely"];
const LAUGH: &[&str] = &["haha", "ahah", "ahaha", "hehe", "lol", "😂", "🤣", "che ridere"];

/// What the words say of how the person feels: lists of words in Italian
/// and English, capitals and exclamation marks. No trained model: cheap, and
/// only a hint.
pub fn from_words(text: &str) -> Mood {
    let low = text.to_lowercase();
    let has = |list: &[&str]| list.iter().any(|w| contains_word(&low, w));
    let mut m = Mood { praise: has(PRAISE), thanks: has(THANKS), rude: has(RUDE), laugh: has(LAUGH), ..Mood::default() };
    let mut v: f32 = 0.0;
    if m.praise || m.thanks {
        v += 0.4;
    }
    if has(HAPPY) {
        v += 0.4;
    }
    if m.laugh {
        v += 0.3;
    }
    if m.rude {
        v -= 0.8;
    }
    if has(UPSET) {
        v -= 0.5;
    }
    if has(SAD) {
        v -= 0.4;
    }
    let letters: Vec<char> = text.chars().filter(|c| c.is_alphabetic()).collect();
    let upper = letters.iter().filter(|c| c.is_uppercase()).count();
    let shouting = letters.len() >= 6 && upper * 10 >= letters.len() * 7;
    let bangs = text.matches('!').count();
    m.arousal = (if shouting { 0.6 } else { 0.0 } + 0.15 * bangs.min(3) as f32 + if m.rude { 0.3 } else { 0.0 }).min(1.0);
    m.valence = v.clamp(-1.0, 1.0);
    m
}

fn contains_word(text: &str, word: &str) -> bool {
    text.match_indices(word).any(|(i, _)| {
        let before = text[..i].chars().next_back().is_none_or(|c| !c.is_alphanumeric());
        let after = text[i + word.len()..].chars().next().is_none_or(|c| !c.is_alphanumeric());
        before && after
    })
}

/// The voice against how the person usually talks: louder, higher and
/// faster is more agitated; quieter, lower and slower, more down.
pub fn from_voice(mood: &mut Mood, usual: &super::voice::Usual, loudness: f32, pitch: Option<f32>, pace: Option<f32>) {
    if usual.heard < 5 {
        return;
    }
    let mut lift = 0.0;
    lift += ((loudness - usual.loudness) / 6.0).clamp(-1.0, 1.0);
    if let (Some(p), true) = (pitch, usual.pitch > 0.0) {
        lift += ((p / usual.pitch - 1.0) / 0.15).clamp(-1.0, 1.0);
    }
    if let (Some(p), true) = (pace, usual.pace > 0.0) {
        lift += ((p / usual.pace - 1.0) / 0.2).clamp(-1.0, 1.0);
    }
    let lift = lift / 3.0;
    if lift > 0.0 {
        mood.arousal = (mood.arousal + lift).min(1.0);
    } else {
        // A flat, quiet, slow voice: down.
        mood.valence = (mood.valence + lift * 0.5).max(-1.0);
    }
}

/// The advice the prompt gets, when the mood says something.
pub fn advice(mood: &Mood) -> Option<&'static str> {
    Some(match (mood.valence, mood.arousal) {
        (v, a) if v <= -0.4 && a >= 0.4 => "They sound upset or angry: stay calm, be brief, don't joke, don't argue.",
        (v, _) if v <= -0.3 => "They sound down or tired: be gentle and warm, keep it simple.",
        (_, a) if a >= 0.5 => "They sound in a hurry or excited: get to the point at once.",
        (v, _) if v >= 0.5 => "They're in a good mood: a light touch of humour is welcome.",
        _ => return None,
    })
}

/// The board's mood event a turn gives (firmware `life_mood.c`'s feelings).
pub fn board_feel(mood: &Mood, face: Option<&str>) -> Option<&'static str> {
    if mood.rude {
        return Some("scolded");
    }
    if mood.praise || mood.thanks {
        return Some("praised");
    }
    if mood.laugh || matches!(face, Some("giggle" | "wink")) {
        return Some("amused");
    }
    None
}

impl Memory {
    /// What Yoru feels for `person` now.
    pub fn feelings(&self, person: i64) -> Feelings {
        let f: Option<Feelings> = self.json::<Option<Feelings>>(person, "feelings");
        f.unwrap_or_default().at(now_ms())
    }

    /// A turn with `person`: what they said (`mood`) and the face Yoru made.
    pub fn feel(&self, person: i64, mood: &Mood, face: Option<&str>) -> Feelings {
        let mut f = self.feelings(person);
        f.after_words(mood);
        if let Some(face) = face {
            f.after_face(face);
        }
        self.set_json(person, "feelings", &f);
        f
    }
}

/// `[face NAME]` at the head of a reply: the face, and the reply without it.
/// Only the faces in [`FACES`]; anything else is left as it is.
pub fn take_face(reply: &str) -> (Option<&'static str>, &str) {
    let trimmed = reply.trim_start();
    let Some(rest) = trimmed.strip_prefix("[face ") else { return (None, reply) };
    let Some(end) = rest.find(']') else { return (None, reply) };
    let name = rest[..end].trim().to_lowercase();
    match FACES.iter().find(|f| **f == name) {
        Some(face) => (Some(face), rest[end + 1..].trim_start()),
        None => (None, reply),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quick_feelings_fade_and_slow_ones_stay() {
        let mut f = Feelings::default();
        f.after_words(&from_words("sei proprio stupido"));
        assert!(f.hurt > 0.3 && f.irritation > 0.2 && f.trust < 0.5);
        let later = f.at(f.at + 10 * 60_000);
        assert!((later.irritation - f.irritation / 2.0).abs() < 0.01);
        assert!(later.hurt > f.hurt * 0.85);
        assert_eq!(later.trust, f.trust);
        assert!(f.line("Ettore").contains("hurt"));
    }

    #[test]
    fn words_tell_the_mood() {
        let m = from_words("GRAZIE MILLE, PERFETTO!!");
        assert!(m.thanks && m.valence > 0.5 && m.arousal > 0.5);
        assert!(from_words("non funziona niente, basta").valence < 0.0);
        assert!(!from_words("ha fatto un grande lavoro").rude);
        assert_eq!(advice(&from_words("sei inutile, stai zitto!")), Some("They sound upset or angry: stay calm, be brief, don't joke, don't argue."));
        assert!(advice(&from_words("che ore sono")).is_none());
    }

    #[test]
    fn the_face_tag_comes_off() {
        assert_eq!(take_face("[face happy] Ciao!"), (Some("happy"), "Ciao!"));
        assert_eq!(take_face("[face nope] Ciao"), (None, "[face nope] Ciao"));
        assert_eq!(take_face("Ciao"), (None, "Ciao"));
    }

    #[test]
    fn feelings_are_kept_per_person() {
        let memory = Memory::open(None);
        let user = memory.user();
        let sofia = memory.resolve("Sofia", Some("person")).unwrap();
        memory.feel(sofia, &from_words("sei inutile"), None);
        assert!(memory.feelings(sofia).hurt > 0.3);
        assert_eq!(memory.feelings(user).hurt, 0.0);
    }
}
