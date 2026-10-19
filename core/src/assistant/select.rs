//! Which connector tools a brain without tool search gets for a request.
//!
//! Claude loads the connectors' tools itself, through tool search. Groq has
//! nothing like it, and its free plan allows about 8,000 tokens a minute:
//! every tool's description and schema would not fit. So the request picks
//! them here: a BM25 score of each tool's service, name and description
//! against what the user said (and the turns just before), and the best few
//! go. The tools a conversation already used stay, so a follow-up ("and
//! reply to it") still finds them.
//!
//! The user often speaks Italian and the tools describe themselves in
//! English: a few common words are mapped to English, and a word matches
//! another that starts with it ("calendar", "calendario").

use std::collections::{HashMap, HashSet};

use super::ExternalTool;

/// BM25's usual constants.
const K1: f64 = 1.2;
const B: f64 = 0.75;
/// The shortest prefix that counts as the same word.
const PREFIX_MIN: usize = 4;

/// Words that say nothing about which tool is needed.
const STOP: &[&str] = &[
    "the", "and", "for", "you", "your", "with", "that", "this", "what", "from", "are", "have", "has", "can", "any", "all", "about", "into", "when", "will",
    "would", "there", "does", "please", "tell", "give", "show", "che", "chi", "con", "per", "una", "uno", "del", "della", "delle", "dei", "degli", "gli",
    "le", "il", "lo", "la", "di", "da", "in", "su", "mi", "ho", "hai", "ha", "cosa", "come", "sono", "mio", "mia", "miei", "mie", "tuo", "tua", "nel",
    "nella", "alla", "allo", "agli", "alle", "dal", "dalla", "non", "anche", "poi", "dove", "quando", "quale", "quali", "qualche", "ci", "si", "ne",
    "it", "is", "of", "to", "a", "an", "on", "me", "my", "do", "be", "or", "by", "at", "as",
];

/// Italian words and the English ones the tools use instead.
const WORDS: &[(&str, &str)] = &[
    ("posta", "mail email"),
    ("mail", "email"),
    ("email", "mail"),
    ("messaggio", "message"),
    ("messaggi", "message"),
    ("riunione", "meeting event"),
    ("riunioni", "meeting event"),
    ("appuntamento", "event meeting"),
    ("appuntamenti", "event meeting"),
    ("impegni", "event calendar"),
    ("agenda", "calendar event"),
    ("oggi", "today"),
    ("domani", "tomorrow"),
    ("settimana", "week"),
    ("documento", "document doc page"),
    ("documenti", "document doc page"),
    ("pagina", "page"),
    ("pagine", "page"),
    ("cartella", "folder"),
    ("progetto", "project"),
    ("progetti", "project"),
    ("compito", "task issue"),
    ("compiti", "task issue"),
    ("attività", "task"),
    ("problema", "issue"),
    ("problemi", "issue"),
    ("ticket", "issue"),
    ("commento", "comment"),
    ("commenti", "comment"),
    ("cerca", "search find"),
    ("trova", "search find"),
    ("leggi", "read get"),
    ("elenca", "list"),
    ("manda", "send"),
    ("invia", "send"),
    ("scrivi", "write send create"),
    ("rispondi", "reply"),
    ("crea", "create"),
    ("aggiungi", "add create"),
    ("cancella", "delete"),
    ("elimina", "delete"),
    ("sposta", "move"),
    ("chiudi", "close"),
    ("aggiorna", "update"),
    ("nuove", "new unread"),
    ("nuovi", "new unread"),
    ("novità", "new unread recent"),
    ("letto", "read"),
    ("lette", "read"),
    ("assegnato", "assigned assignee"),
    ("assegnati", "assigned assignee"),
    ("file", "file document"),
];

/// The words of `text`, lowercased, without the ones that say nothing.
fn words(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    // `list_issues`, `getEvent`, `gmail__search`: each part a word.
    let mut spaced = String::with_capacity(text.len());
    let mut prev_lower = false;
    for c in text.chars() {
        if c.is_uppercase() && prev_lower {
            spaced.push(' ');
        }
        prev_lower = c.is_lowercase();
        spaced.push(c);
    }
    for w in spaced.to_lowercase().split(|c: char| !c.is_alphanumeric()) {
        if w.chars().count() >= 2 && !STOP.contains(&w) {
            out.push(w.to_string());
        }
    }
    out
}

/// The query's words, with the English ones its Italian words stand for.
fn query_words(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for w in words(text) {
        if let Some((_, en)) = WORDS.iter().find(|(it, _)| *it == w) {
            out.extend(en.split(' ').map(str::to_string));
        }
        out.push(w);
    }
    out
}

/// Whether query word `q` names document word `d`.
fn same(q: &str, d: &str) -> bool {
    if q == d {
        return true;
    }
    let (short, long) = if q.len() <= d.len() { (q, d) } else { (d, q) };
    short.chars().count() >= PREFIX_MIN && long.starts_with(short)
}

/// The words a tool is found by: its service and name count twice.
fn document(tool: &ExternalTool) -> Vec<String> {
    let mut doc = Vec::new();
    for _ in 0..2 {
        doc.extend(words(&tool.service));
        doc.extend(words(&tool.name));
    }
    doc.extend(words(&tool.description));
    doc
}

/// Each tool's BM25 score against `query`, in `tools`' order.
pub fn scores(tools: &[ExternalTool], query: &str) -> Vec<f64> {
    let docs: Vec<Vec<String>> = tools.iter().map(document).collect();
    let n = docs.len() as f64;
    let average = docs.iter().map(Vec::len).sum::<usize>() as f64 / n.max(1.0);
    let terms: HashSet<String> = query_words(query).into_iter().collect();
    // How many tools each query word names.
    let df: HashMap<&String, usize> = terms.iter().map(|t| (t, docs.iter().filter(|d| d.iter().any(|w| same(t, w))).count())).collect();
    docs.iter()
        .map(|doc| {
            let len = doc.len() as f64;
            terms
                .iter()
                .map(|t| {
                    let tf = doc.iter().filter(|w| same(t, w)).count() as f64;
                    if tf == 0.0 {
                        return 0.0;
                    }
                    let df = df[t] as f64;
                    let idf = ((n - df + 0.5) / (df + 0.5) + 1.0).ln();
                    idf * tf * (K1 + 1.0) / (tf + K1 * (1.0 - B + B * len / average.max(1.0)))
                })
                .sum()
        })
        .collect()
}

/// The tools for a request: those `used` already, then the best `keep` of
/// the rest for `query`. When nothing matches (a "what's new?"), the
/// tools that read, a few per service, so the model can still look.
pub fn pick<'a>(tools: &'a [ExternalTool], query: &str, used: &[String], keep: usize) -> Vec<&'a ExternalTool> {
    let mut chosen: Vec<&ExternalTool> = tools.iter().filter(|t| used.contains(&t.name)).collect();
    let scores = scores(tools, query);
    let mut ranked: Vec<(usize, f64)> = scores.into_iter().enumerate().filter(|(i, s)| *s > 0.0 && !used.contains(&tools[*i].name)).collect();
    ranked.sort_by(|a, b| b.1.total_cmp(&a.1));
    chosen.extend(ranked.into_iter().take(keep).map(|(i, _)| &tools[i]));
    if chosen.is_empty() {
        chosen = readers(tools, keep / 2);
    }
    chosen
}

/// Up to `max` tools that only read, taking each service's in turn and
/// those that list or search first.
fn readers(tools: &[ExternalTool], max: usize) -> Vec<&ExternalTool> {
    let looks = |t: &ExternalTool| ["search", "list", "unread", "recent"].iter().any(|w| t.name.to_lowercase().contains(w));
    let mut services: Vec<&str> = Vec::new();
    for t in tools {
        if !services.contains(&t.service.as_str()) {
            services.push(&t.service);
        }
    }
    let mut queues: Vec<Vec<&ExternalTool>> = services
        .iter()
        .map(|s| {
            let mut q: Vec<&ExternalTool> = tools.iter().filter(|t| t.service == *s && !t.acts).collect();
            q.sort_by_key(|t| !looks(t));
            q.reverse();
            q
        })
        .collect();
    let mut out = Vec::new();
    while out.len() < max && queues.iter().any(|q| !q.is_empty()) {
        for q in &mut queues {
            if out.len() < max
                && let Some(t) = q.pop()
            {
                out.push(t);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn tool(service: &str, name: &str, description: &str, acts: bool) -> ExternalTool {
        ExternalTool { name: name.into(), service: service.into(), description: description.into(), input_schema: json!({"type": "object"}), acts }
    }

    fn catalog() -> Vec<ExternalTool> {
        vec![
            tool("Gmail", "gmail__search_threads", "Search the mail threads in the user's mailbox with a Gmail query.", false),
            tool("Gmail", "gmail__send_message", "Send an email message.", true),
            tool("Gmail", "gmail__get_thread", "Read one mail thread.", false),
            tool("Calendar", "calendar__list_events", "List the events of a calendar between two dates.", false),
            tool("Calendar", "calendar__create_event", "Create an event in a calendar.", true),
            tool("Linear", "linear__list_issues", "List issues assigned to you or matching a filter.", false),
            tool("Linear", "linear__create_issue", "Create an issue in a team.", true),
            tool("Notion", "notion__search", "Search pages and databases of the workspace.", false),
            tool("Notion", "notion__createPage", "Create a page.", true),
        ]
    }

    fn names<'a>(picked: &[&'a ExternalTool]) -> Vec<&'a str> {
        picked.iter().map(|t| t.name.as_str()).collect()
    }

    #[test]
    fn words_split_names_and_drop_noise() {
        assert_eq!(words("notion__createPage"), ["notion", "create", "page"]);
        assert_eq!(words("What's on my calendar?"), ["calendar"]);
        assert!(query_words("che riunioni ho domani").contains(&"meeting".to_string()));
    }

    #[test]
    fn the_best_tools_come_first() {
        let tools = catalog();
        let picked = pick(&tools, "Send an email to Marco", &[], 3);
        assert_eq!(picked[0].name, "gmail__send_message");
        assert!(names(&picked).iter().all(|n| n.starts_with("gmail")));
        // In Italian, through the word list and prefixes.
        let picked = pick(&tools, "Cosa ho in calendario domani?", &[], 2);
        assert_eq!(picked[0].service, "Calendar");
        let picked = pick(&tools, "crea un problema su Linear per la scheda", &[], 2);
        assert_eq!(picked[0].name, "linear__create_issue");
        let picked = pick(&tools, "cerca nelle pagine di Notion", &[], 2);
        assert!(names(&picked).contains(&"notion__search"));
    }

    #[test]
    fn used_tools_stay_and_nothing_matching_still_reads() {
        let tools = catalog();
        let used = vec!["gmail__get_thread".to_string()];
        let picked = pick(&tools, "and the issues?", &used, 2);
        assert_eq!(picked[0].name, "gmail__get_thread");
        assert!(names(&picked).contains(&"linear__list_issues"));
        // "Anything new?" names no tool: the readers, one per service first.
        let picked = pick(&tools, "Ciao, tutto bene?", &[], 8);
        assert_eq!(picked.len(), 4);
        assert!(picked.iter().all(|t| !t.acts));
        let services: HashSet<&str> = picked.iter().map(|t| t.service.as_str()).collect();
        assert_eq!(services.len(), 4);
        assert!(pick(&[], "mail", &[], 8).is_empty());
    }
}
