//! How people are related. Only a few canonical relations are stored
//! (`parent`, `partner`, `sibling`, `friend`, `colleague`); words like
//! mother-in-law are computed from them, from the side of whoever asks
//! (Bip's idea): "my wife" and "Sofia's mother" lead to the same node.

use std::collections::{HashMap, HashSet, VecDeque};

/// The relations kept, as the tools and the sleep name them. `parent` is
/// directed (a is b's parent); the others go both ways.
pub const KINDS: &[&str] = &["parent", "child", "partner", "sibling", "friend", "colleague"];

/// One step from a person to another.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Step {
    Parent,
    Child,
    Partner,
    Sibling,
    Friend,
    Colleague,
}

impl Step {
    /// The stored kind and whether a and b swap: `child` is kept as `parent`.
    pub fn stored(kind: &str) -> Option<(&'static str, bool)> {
        Some(match kind {
            "parent" => ("parent", false),
            "child" => ("parent", true),
            "partner" => ("partner", false),
            "sibling" => ("sibling", false),
            "friend" => ("friend", false),
            "colleague" | "works_with" => ("colleague", false),
            _ => return None,
        })
    }

    fn word(self, gender: Option<&str>) -> &'static str {
        let g = gender.unwrap_or_default();
        let (f, m, n) = match self {
            Step::Parent => ("mother", "father", "parent"),
            Step::Child => ("daughter", "son", "child"),
            Step::Partner => ("wife", "husband", "partner"),
            Step::Sibling => ("sister", "brother", "sibling"),
            Step::Friend => ("friend", "friend", "friend"),
            Step::Colleague => ("colleague", "colleague", "colleague"),
        };
        match g {
            "f" => f,
            "m" => m,
            _ => n,
        }
    }
}

/// A relation as stored: `a` is `kind` of `b`.
#[derive(Debug, Clone)]
pub struct Edge {
    pub a: i64,
    pub kind: String,
    pub b: i64,
}

/// The steps out of each person.
fn graph(edges: &[Edge]) -> HashMap<i64, Vec<(i64, Step)>> {
    let mut out: HashMap<i64, Vec<(i64, Step)>> = HashMap::new();
    for e in edges {
        let (ab, ba) = match e.kind.as_str() {
            // a is b's parent: from b, a is a parent; from a, b is a child.
            "parent" => (Step::Child, Step::Parent),
            "partner" => (Step::Partner, Step::Partner),
            "sibling" => (Step::Sibling, Step::Sibling),
            "friend" => (Step::Friend, Step::Friend),
            "colleague" => (Step::Colleague, Step::Colleague),
            _ => continue,
        };
        out.entry(e.a).or_default().push((e.b, ab));
        out.entry(e.b).or_default().push((e.a, ba));
    }
    out
}

/// The shortest way from `from` to `to`, at most three steps: the people
/// passed and the step into each.
pub fn path(edges: &[Edge], from: i64, to: i64) -> Option<Vec<(i64, Step)>> {
    if from == to {
        return None;
    }
    let graph = graph(edges);
    let mut seen = HashSet::from([from]);
    let mut queue = VecDeque::from([(from, Vec::new())]);
    while let Some((at, way)) = queue.pop_front() {
        if way.len() >= 3 {
            continue;
        }
        for &(next, step) in graph.get(&at).into_iter().flatten() {
            if !seen.insert(next) {
                continue;
            }
            let mut longer: Vec<(i64, Step)> = way.clone();
            longer.push((next, step));
            if next == to {
                return Some(longer);
            }
            queue.push_back((next, longer));
        }
    }
    None
}

/// One word for a way of two or three steps, when English has one.
fn compound(steps: &[Step], gender: Option<&str>) -> Option<String> {
    use Step::*;
    let g = gender.unwrap_or_default();
    let pick = |f: &str, m: &str, n: &str| Some(match g { "f" => f, "m" => m, _ => n }.to_string());
    match steps {
        [Parent, Parent] => pick("grandmother", "grandfather", "grandparent"),
        [Child, Child] => pick("granddaughter", "grandson", "grandchild"),
        [Parent, Sibling] => pick("aunt", "uncle", "aunt or uncle"),
        [Sibling, Child] => pick("niece", "nephew", "niece or nephew"),
        [Partner, Parent] => pick("mother-in-law", "father-in-law", "parent-in-law"),
        [Partner, Sibling] | [Sibling, Partner] => pick("sister-in-law", "brother-in-law", "sibling-in-law"),
        [Child, Partner] => pick("daughter-in-law", "son-in-law", "child-in-law"),
        [Parent, Sibling, Child] => pick("cousin", "cousin", "cousin"),
        [Parent, Child] => pick("sister", "brother", "sibling"),
        _ => None,
    }
}

/// What `to` is to `from`, said from `from`'s side: "your mother-in-law",
/// "your wife's friend" (`owner`: "your", or "Marco's").
pub fn label(edges: &[Edge], from: i64, to: i64, owner: &str, gender: &dyn Fn(i64) -> Option<String>) -> Option<String> {
    let way = path(edges, from, to)?;
    let steps: Vec<Step> = way.iter().map(|(_, s)| *s).collect();
    if let Some(word) = compound(&steps, gender(to).as_deref()) {
        return Some(format!("{owner} {word}"));
    }
    let words: Vec<&str> = way.iter().map(|(id, step)| step.word(gender(*id).as_deref())).collect();
    Some(format!("{owner} {}", words.join("'s ")))
}

/// The way a kinship word ("wife", "mother-in-law", "suocera") goes from
/// the person who says it, and the gender it names.
pub fn parse(word: &str) -> Option<(Vec<Step>, Option<&'static str>)> {
    use Step::*;
    let w = word.trim().to_lowercase();
    let w = ["my ", "your ", "mia ", "mio ", "la mia ", "il mio ", "tua ", "tuo "].iter().fold(w, |w, p| w.strip_prefix(p).map(str::to_string).unwrap_or(w));
    let (steps, g): (&[Step], Option<&str>) = match w.as_str() {
        "wife" | "moglie" => (&[Partner], Some("f")),
        "husband" | "marito" => (&[Partner], Some("m")),
        "partner" | "girlfriend" | "fidanzata" | "compagna" => (&[Partner], if w == "partner" { None } else { Some("f") }),
        "boyfriend" | "fidanzato" | "compagno" => (&[Partner], Some("m")),
        "mother" | "mom" | "mum" | "madre" | "mamma" => (&[Parent], Some("f")),
        "father" | "dad" | "padre" | "papà" | "papa" => (&[Parent], Some("m")),
        "daughter" | "figlia" => (&[Child], Some("f")),
        "son" | "figlio" => (&[Child], Some("m")),
        "sister" | "sorella" => (&[Sibling], Some("f")),
        "brother" | "fratello" => (&[Sibling], Some("m")),
        "grandmother" | "grandma" | "nonna" => (&[Parent, Parent], Some("f")),
        "grandfather" | "grandpa" | "nonno" => (&[Parent, Parent], Some("m")),
        "aunt" | "zia" => (&[Parent, Sibling], Some("f")),
        "uncle" | "zio" => (&[Parent, Sibling], Some("m")),
        "mother-in-law" | "suocera" => (&[Partner, Parent], Some("f")),
        "father-in-law" | "suocero" => (&[Partner, Parent], Some("m")),
        "sister-in-law" | "cognata" => (&[Partner, Sibling], Some("f")),
        "brother-in-law" | "cognato" => (&[Partner, Sibling], Some("m")),
        "daughter-in-law" | "nuora" => (&[Child, Partner], Some("f")),
        "son-in-law" | "genero" => (&[Child, Partner], Some("m")),
        _ => return None,
    };
    Some((steps.to_vec(), g))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn e(a: i64, kind: &str, b: i64) -> Edge {
        Edge { a, kind: kind.into(), b }
    }

    #[test]
    fn words_come_from_the_askers_side() {
        // 1 the user, 2 Sofia (wife), 3 Anna (Sofia's mother), 4 Luca (Sofia's friend).
        let edges = [e(1, "partner", 2), e(3, "parent", 2), e(4, "friend", 2)];
        let gender = |id: i64| match id {
            2 | 3 => Some("f".to_string()),
            4 => Some("m".to_string()),
            _ => None,
        };
        assert_eq!(label(&edges, 1, 2, "your", &gender).unwrap(), "your wife");
        assert_eq!(label(&edges, 1, 3, "your", &gender).unwrap(), "your mother-in-law");
        assert_eq!(label(&edges, 2, 3, "Sofia's", &gender).unwrap(), "Sofia's mother");
        assert_eq!(label(&edges, 1, 4, "your", &gender).unwrap(), "your wife's friend");
        assert_eq!(label(&edges, 3, 1, "Anna's", &gender).unwrap(), "Anna's child-in-law");
    }

    #[test]
    fn parses_kinship_words() {
        assert_eq!(parse("my mother-in-law").unwrap(), (vec![Step::Partner, Step::Parent], Some("f")));
        assert_eq!(parse("suocera").unwrap().0.len(), 2);
        assert!(parse("Sofia").is_none());
    }
}
