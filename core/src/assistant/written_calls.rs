//! Tool calls a model wrote as words instead of making them. Small models
//! (Gemini Flash Lite most of all) sometimes answer
//! `Let me look. [tavily__tavily_search(query="news today", topic="news")]`
//! or `print(default_api.get_timers())` with no real call: the turn ended
//! there and the user got the call's text as the answer. Here such a call,
//! to a tool the model was given, is read back into a real one, and a
//! streamed answer holds it back so a voice never reads it out.

use serde_json::{Map, Value};

/// The calls to `names` written in `text`, each with its arguments, and the
/// text without them. None when there is no such call.
pub fn take(text: &str, names: &[String]) -> Option<(String, Vec<(String, Value)>)> {
    let mut calls = Vec::new();
    let mut rest = String::new();
    let mut from = 0;
    let mut i = 0;
    while i < text.len() {
        if let Some((name, open)) = name_at(text, i, names)
            && let Some((arguments, close)) = arguments(text, open)
        {
            let (start, end) = wrapping(text, i, close);
            rest.push_str(&text[from..start]);
            calls.push((name.to_string(), arguments));
            from = end;
            i = end;
            continue;
        }
        i += text[i..].chars().next().map_or(1, char::len_utf8);
    }
    if calls.is_empty() {
        return None;
    }
    rest.push_str(&text[from..]);
    Some((tidy(&rest), calls))
}

/// A known tool's name starting at `i` (alone, not the end of a longer
/// word), and where its `(` is.
fn name_at<'a>(text: &str, i: usize, names: &'a [String]) -> Option<(&'a str, usize)> {
    let before = text[..i].chars().next_back();
    if before.is_some_and(|c| c.is_alphanumeric() || c == '_') {
        return None;
    }
    names.iter().find_map(|name| {
        let after = text[i..].strip_prefix(name.as_str())?;
        let open = i + name.len() + (after.len() - after.trim_start().len());
        text[open..].starts_with('(').then_some((name.as_str(), open))
    })
}

/// The arguments from the `(` at `open` to its `)`: `k="v", n=2` as Python
/// writes them, or one JSON object. With where the `)` ends.
fn arguments(text: &str, open: usize) -> Option<(Value, usize)> {
    let mut p = Parser { s: text, i: open + 1 };
    p.space();
    if p.eat(')') {
        return Some((Value::Object(Map::new()), p.i));
    }
    if p.peek() == Some('{') {
        let value = p.value()?;
        p.space();
        return (value.is_object() && p.eat(')')).then_some((value, p.i));
    }
    let mut map = Map::new();
    loop {
        p.space();
        let key = p.word()?;
        p.space();
        if !(p.eat('=') || p.eat(':')) {
            return None;
        }
        p.space();
        map.insert(key, p.value()?);
        p.space();
        if p.eat(')') {
            return Some((Value::Object(map), p.i));
        }
        if !p.eat(',') {
            return None;
        }
        p.space();
        if p.eat(')') {
            return Some((Value::Object(map), p.i));
        }
    }
}

/// The call with what wraps it: `[...]`, `print(...)`, `default_api.`, a
/// backtick.
fn wrapping(text: &str, start: usize, end: usize) -> (usize, usize) {
    let (mut start, mut end) = (start, end);
    if text[..start].ends_with("default_api.") {
        start -= "default_api.".len();
    }
    for (open, close) in [("print(", ")"), ("[", "]"), ("`", "`")] {
        let before = text[..start].trim_end_matches(' ');
        let after = text[end..].trim_start_matches(' ');
        if before.ends_with(open) && after.starts_with(close) {
            start = before.len() - open.len();
            end = text.len() - after.len() + close.len();
        }
    }
    (start, end)
}

/// The words left: no empty code fence where the calls were, no blank lines
/// piling up.
fn tidy(text: &str) -> String {
    let mut lines: Vec<&str> = Vec::new();
    let raw: Vec<&str> = text.lines().collect();
    let mut i = 0;
    while i < raw.len() {
        let line = raw[i].trim();
        // An opening fence followed by its closing one, with only blanks between.
        if line.starts_with("```") {
            let mut j = i + 1;
            while j < raw.len() && raw[j].trim().is_empty() {
                j += 1;
            }
            if j < raw.len() && raw[j].trim() == "```" {
                i = j + 1;
                continue;
            }
        }
        if !(line.is_empty() && lines.last().is_none_or(|l| l.trim().is_empty())) {
            lines.push(raw[i].trim_end());
        }
        i += 1;
    }
    lines.join("\n").trim().to_string()
}

/// Python's literals and JSON's, as JSON.
struct Parser<'a> {
    s: &'a str,
    i: usize,
}

impl Parser<'_> {
    fn peek(&self) -> Option<char> {
        self.s[self.i..].chars().next()
    }

    fn eat(&mut self, c: char) -> bool {
        let ate = self.peek() == Some(c);
        if ate {
            self.i += c.len_utf8();
        }
        ate
    }

    fn space(&mut self) {
        while self.peek().is_some_and(char::is_whitespace) {
            self.i += 1;
        }
    }

    fn word(&mut self) -> Option<String> {
        if let Some(q @ ('"' | '\'')) = self.peek() {
            self.i += 1;
            return self.string(q);
        }
        let start = self.i;
        while self.peek().is_some_and(|c| c.is_alphanumeric() || c == '_') {
            self.i += self.peek().map_or(1, char::len_utf8);
        }
        (self.i > start).then(|| self.s[start..self.i].to_string())
    }

    fn value(&mut self) -> Option<Value> {
        match self.peek()? {
            q @ ('"' | '\'') => {
                self.i += 1;
                self.string(q).map(Value::String)
            }
            '[' => {
                self.i += 1;
                let mut items = Vec::new();
                loop {
                    self.space();
                    if self.eat(']') {
                        return Some(Value::Array(items));
                    }
                    items.push(self.value()?);
                    self.space();
                    if !self.eat(',') {
                        self.space();
                        return self.eat(']').then_some(Value::Array(items));
                    }
                }
            }
            '{' => {
                self.i += 1;
                let mut map = Map::new();
                loop {
                    self.space();
                    if self.eat('}') {
                        return Some(Value::Object(map));
                    }
                    let key = self.word()?;
                    self.space();
                    if !self.eat(':') {
                        return None;
                    }
                    self.space();
                    map.insert(key, self.value()?);
                    self.space();
                    if !self.eat(',') {
                        self.space();
                        return self.eat('}').then_some(Value::Object(map));
                    }
                }
            }
            _ => {
                let start = self.i;
                while self.peek().is_some_and(|c| c.is_alphanumeric() || matches!(c, '.' | '-' | '+' | '_')) {
                    self.i += 1;
                }
                match &self.s[start..self.i] {
                    "True" | "true" => Some(Value::Bool(true)),
                    "False" | "false" => Some(Value::Bool(false)),
                    "None" | "null" => Some(Value::Null),
                    number => serde_json::from_str::<serde_json::Number>(number).ok().map(Value::Number),
                }
            }
        }
    }

    /// A string's body after its opening quote `q`, escapes read.
    fn string(&mut self, q: char) -> Option<String> {
        let mut out = String::new();
        let mut chars = self.s[self.i..].char_indices();
        while let Some((at, c)) = chars.next() {
            match c {
                '\\' => {
                    let (_, e) = chars.next()?;
                    out.push(match e {
                        'n' => '\n',
                        't' => '\t',
                        other => other,
                    });
                }
                c if c == q => {
                    self.i += at + c.len_utf8();
                    return Some(out);
                }
                c => out.push(c),
            }
        }
        None
    }
}

/// A streamed answer with what may be a written call held back: from a `[`
/// until it is clearly not `[name(`. A call that turned out to be one is
/// never let through; [`take`] reads it from the whole answer.
#[derive(Default)]
pub struct Holder {
    held: String,
    /// What was held is a call: everything after it stays back too.
    call: bool,
}

impl Holder {
    /// `text` as it may go on now.
    pub fn push(&mut self, text: &str) -> String {
        let mut out = String::new();
        for c in text.chars() {
            if self.call {
                continue;
            }
            if self.held.is_empty() {
                if c == '[' {
                    self.held.push(c);
                } else {
                    out.push(c);
                }
                continue;
            }
            self.held.push(c);
            let inside = &self.held[1..];
            let name_len = inside.find(|c: char| !(c.is_alphanumeric() || matches!(c, '_' | '.'))).unwrap_or(inside.len());
            match inside[name_len..].chars().next() {
                // Still a name, or nothing yet.
                None => {}
                Some('(') if name_len > 0 => {
                    self.call = true;
                    self.held.clear();
                }
                Some(_) => out.push_str(&std::mem::take(&mut self.held)),
            }
        }
        out
    }

    /// What is still held once the answer ended, unless it was a call.
    pub fn finish(&mut self) -> String {
        let held = std::mem::take(&mut self.held);
        let call = std::mem::take(&mut self.call);
        if call { String::new() } else { held }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn names() -> Vec<String> {
        vec!["tavily__tavily_search".into(), "get_timers".into(), "notify".into()]
    }

    #[test]
    fn a_bracketed_call_is_read_and_taken_out() {
        let text = "Vediamo un po'...\n\n*(Sto cercando le notizie del giorno)*[tavily__tavily_search(query=\"ultime notizie oggi\",topic=\"news\")]";
        let (rest, calls) = take(text, &names()).unwrap();
        assert_eq!(rest, "Vediamo un po'...\n\n*(Sto cercando le notizie del giorno)*");
        assert_eq!(calls, vec![("tavily__tavily_search".to_string(), json!({"query": "ultime notizie oggi", "topic": "news"}))]);
    }

    #[test]
    fn python_and_json_forms_are_read() {
        let text = "```tool_code\nprint(default_api.get_timers())\n```\nnotify({\"text\": \"hi\", \"urgency\": \"low\"}) and [notify(text='a \\'b\\'', tags=['x', 2], loud=True)]";
        let (rest, calls) = take(text, &names()).unwrap();
        assert_eq!(rest, "and");
        assert_eq!(calls[0], ("get_timers".to_string(), json!({})));
        assert_eq!(calls[1], ("notify".to_string(), json!({"text": "hi", "urgency": "low"})));
        assert_eq!(calls[2], ("notify".to_string(), json!({"text": "a 'b'", "tags": ["x", 2], "loud": true})));
    }

    #[test]
    fn words_that_only_look_like_calls_stay() {
        assert!(take("I'll notify you (later), see [the docs](https://x.y).", &names()).is_none());
        assert!(take("unknown_tool(a=1)", &names()).is_none());
        assert!(take("my_notify(a=1)", &names()).is_none());
        assert!(take("notify(a=", &names()).is_none());
    }

    #[test]
    fn a_streamed_call_is_held_back() {
        let mut h = Holder::default();
        let mut said = String::new();
        for piece in ["Look", "ing. [tav", "ily__tavily_search(query=\"x\")]", " more"] {
            said.push_str(&h.push(piece));
        }
        said.push_str(&h.finish());
        assert_eq!(said, "Looking. ");
    }

    #[test]
    fn brackets_that_arent_calls_go_on() {
        let mut h = Holder::default();
        let mut said = String::new();
        for piece in ["See [the", " docs] and [x", "]"] {
            said.push_str(&h.push(piece));
        }
        said.push_str(&h.finish());
        assert_eq!(said, "See [the docs] and [x]");
    }
}
