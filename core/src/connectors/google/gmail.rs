//! Gmail's tools, on the Gmail API (`gmail.googleapis.com/gmail/v1`).
//!
//! Threads are the unit, as in Gmail itself: search finds threads, reading
//! one gives its messages as plain text, and the actions (read, archive,
//! label, trash) apply to the whole thread. Messages are written as plain
//! text and sent raw (RFC 5322, base64url), the only way the API takes them.

use base64::Engine;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD, URL_SAFE_NO_PAD_INDIFFERENT};
use reqwest::Method;
use serde_json::{Value, json};

use super::{Api, Args, ToolDef, ToolResult, pretty, segment};

const BASE: &str = "https://gmail.googleapis.com/gmail/v1/users/me";
const DEFAULT_RESULTS: u64 = 10;
const MAX_RESULTS: u64 = 25;
/// The text of one message the model gets, at most (characters).
const MAX_BODY: usize = 6000;

pub(super) fn tools() -> Vec<ToolDef> {
    let thread_id = json!({"type": "string", "description": "The thread's id, from search_threads."});
    vec![
        ToolDef {
            name: "search_threads",
            description: "Search the user's mail with a Gmail query (from:, to:, subject:, is:unread, newer_than:2d, label:, has:attachment...). \
                          Gives each thread's id, subject, sender, date, snippet and whether it's unread. Without a query: the inbox.",
            properties: json!({
                "query": {"type": "string", "description": "A Gmail search query, as in Gmail's search box. Default: in:inbox."},
                "max_results": {"type": "integer", "description": "How many threads, 1 to 25 (default 10)."}
            }),
            required: &[],
            read_only: true,
            destructive: false,
        },
        ToolDef {
            name: "get_thread",
            description: "Read a mail thread: every message with its sender, recipients, date, text and attachment names.",
            properties: json!({"thread_id": thread_id}),
            required: &["thread_id"],
            read_only: true,
            destructive: false,
        },
        ToolDef {
            name: "list_labels",
            description: "List the user's Gmail labels (system and their own).",
            properties: json!({}),
            required: &[],
            read_only: true,
            destructive: false,
        },
        ToolDef {
            name: "send_message",
            description: "Send a new plain-text email from the user's account.",
            properties: json!({
                "to": {"type": "array", "items": {"type": "string"}, "description": "Recipients' addresses."},
                "subject": {"type": "string"},
                "body": {"type": "string", "description": "The text of the message."},
                "cc": {"type": "array", "items": {"type": "string"}},
                "bcc": {"type": "array", "items": {"type": "string"}}
            }),
            required: &["to", "subject", "body"],
            read_only: false,
            destructive: false,
        },
        ToolDef {
            name: "reply",
            description: "Reply to the last message of a thread, in the same thread. With draft: true it's saved as a draft instead of sent.",
            properties: json!({
                "thread_id": thread_id,
                "body": {"type": "string", "description": "The text of the reply."},
                "reply_all": {"type": "boolean", "description": "Reply to every recipient too (default false)."},
                "draft": {"type": "boolean", "description": "Save as a draft, don't send (default false)."}
            }),
            required: &["thread_id", "body"],
            read_only: false,
            destructive: false,
        },
        ToolDef {
            name: "create_draft",
            description: "Save a new plain-text email as a draft, for the user to review and send from Gmail.",
            properties: json!({
                "to": {"type": "array", "items": {"type": "string"}},
                "subject": {"type": "string"},
                "body": {"type": "string"},
                "cc": {"type": "array", "items": {"type": "string"}}
            }),
            required: &["subject", "body"],
            read_only: false,
            destructive: false,
        },
        ToolDef {
            name: "mark_read",
            description: "Mark a thread as read, or as unread with read: false.",
            properties: json!({"thread_id": thread_id, "read": {"type": "boolean", "description": "Default true."}}),
            required: &["thread_id"],
            read_only: false,
            destructive: false,
        },
        ToolDef {
            name: "archive_thread",
            description: "Archive a thread: it leaves the inbox and stays in All Mail.",
            properties: json!({"thread_id": thread_id}),
            required: &["thread_id"],
            read_only: false,
            destructive: false,
        },
        ToolDef {
            name: "label_thread",
            description: "Add or remove labels on a thread, by name or id (see list_labels).",
            properties: json!({
                "thread_id": thread_id,
                "add": {"type": "array", "items": {"type": "string"}},
                "remove": {"type": "array", "items": {"type": "string"}}
            }),
            required: &["thread_id"],
            read_only: false,
            destructive: false,
        },
        ToolDef {
            name: "trash_thread",
            description: "Move a thread to the trash (Gmail deletes it for good after 30 days).",
            properties: json!({"thread_id": thread_id}),
            required: &["thread_id"],
            read_only: false,
            destructive: true,
        },
    ]
}

pub(super) async fn call(api: &Api, name: &str, args: &Args) -> ToolResult {
    match name {
        "search_threads" => search(api, args).await,
        "get_thread" => get_thread(api, args.need("thread_id")?).await,
        "list_labels" => list_labels(api).await,
        "send_message" => send_new(api, args, false).await,
        "create_draft" => send_new(api, args, true).await,
        "reply" => reply(api, args).await,
        "mark_read" => {
            let label = vec!["UNREAD".to_string()];
            let (add, remove) = if args.bool("read").unwrap_or(true) { (vec![], label) } else { (label, vec![]) };
            modify(api, args.need("thread_id")?, add, remove).await
        }
        "archive_thread" => modify(api, args.need("thread_id")?, vec![], vec!["INBOX".into()]).await,
        "label_thread" => {
            let (add, remove) = (label_ids(api, args.list("add")).await?, label_ids(api, args.list("remove")).await?);
            modify(api, args.need("thread_id")?, add, remove).await
        }
        "trash_thread" => {
            let id = args.need("thread_id")?;
            api.send(Method::POST, &format!("{BASE}/threads/{}/trash", segment(id)), &[], None).await.map_err(|e| e.describe())?;
            Ok(format!("Thread {id} moved to the trash."))
        }
        other => Err(format!("Gmail has no tool {other:?}")),
    }
}

async fn search(api: &Api, args: &Args) -> ToolResult {
    let query = args.str("query").unwrap_or("in:inbox").to_string();
    let max = args.int("max_results").unwrap_or(DEFAULT_RESULTS).clamp(1, MAX_RESULTS);
    let list = api.get(&format!("{BASE}/threads"), &[("q", query.clone()), ("maxResults", max.to_string())]).await.map_err(|e| e.describe())?;
    let ids: Vec<&str> = list["threads"].as_array().into_iter().flatten().filter_map(|t| t["id"].as_str()).collect();
    if ids.is_empty() {
        return Ok(format!("No threads match {query:?}."));
    }
    let headers = ["From", "Subject", "Date"].map(|h| ("metadataHeaders", h.to_string()));
    let mut query_params = vec![("format", "metadata".to_string())];
    query_params.extend(headers);
    let fetches = ids.iter().map(|id| api.get_owned(format!("{BASE}/threads/{}", segment(id)), query_params.clone()));
    let threads = futures::future::join_all(fetches).await;
    let summaries: Vec<Value> = threads.into_iter().filter_map(Result::ok).map(|t| thread_summary(&t)).collect();
    Ok(pretty(&json!({"query": query, "threads": summaries})))
}

impl Api {
    /// [`Api::get`] owning its arguments, for calls made side by side.
    async fn get_owned(&self, url: String, query: Vec<(&'static str, String)>) -> Result<Value, super::ApiError> {
        self.get(&url, &query).await
    }
}

/// A thread in a list: the first subject, the last sender and date.
fn thread_summary(thread: &Value) -> Value {
    let messages = thread["messages"].as_array().cloned().unwrap_or_default();
    let first = messages.first().cloned().unwrap_or_default();
    let last = messages.last().cloned().unwrap_or_default();
    let unread = messages.iter().any(|m| has_label(m, "UNREAD"));
    json!({
        "thread_id": thread["id"],
        "subject": header(&first, "Subject"),
        "from": header(&last, "From"),
        "date": header(&last, "Date"),
        "messages": messages.len(),
        "unread": unread,
        "snippet": html_unescape(last["snippet"].as_str().unwrap_or_default()),
    })
}

async fn get_thread(api: &Api, id: &str) -> ToolResult {
    let thread = api.get(&format!("{BASE}/threads/{}", segment(id)), &[("format", "full".into())]).await.map_err(|e| e.describe())?;
    let messages: Vec<Value> = thread["messages"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|m| {
            let mut attachments = Vec::new();
            collect_attachments(&m["payload"], &mut attachments);
            json!({
                "message_id": m["id"],
                "from": header(m, "From"),
                "to": header(m, "To"),
                "cc": header(m, "Cc"),
                "date": header(m, "Date"),
                "subject": header(m, "Subject"),
                "unread": has_label(m, "UNREAD"),
                "text": truncate(&message_text(&m["payload"]), MAX_BODY),
                "attachments": attachments,
            })
        })
        .collect();
    Ok(pretty(&json!({"thread_id": id, "messages": messages})))
}

async fn list_labels(api: &Api) -> ToolResult {
    let labels = api.get(&format!("{BASE}/labels"), &[]).await.map_err(|e| e.describe())?;
    let list: Vec<Value> = labels["labels"].as_array().into_iter().flatten().map(|l| json!({"id": l["id"], "name": l["name"], "type": l["type"]})).collect();
    Ok(pretty(&json!({"labels": list})))
}

/// Label names to ids; ids and system labels (`INBOX`, `STARRED`) pass as they are.
async fn label_ids(api: &Api, wanted: Vec<String>) -> Result<Vec<String>, String> {
    if wanted.is_empty() {
        return Ok(wanted);
    }
    let labels = api.get(&format!("{BASE}/labels"), &[]).await.map_err(|e| e.describe())?;
    let all = labels["labels"].as_array().cloned().unwrap_or_default();
    wanted
        .into_iter()
        .map(|w| {
            all.iter()
                .find(|l| l["id"].as_str() == Some(&w) || l["name"].as_str().is_some_and(|n| n.eq_ignore_ascii_case(&w)))
                .and_then(|l| l["id"].as_str().map(str::to_string))
                .ok_or_else(|| format!("no label {w:?}; list_labels says which there are"))
        })
        .collect()
}

async fn modify(api: &Api, id: &str, add: Vec<String>, remove: Vec<String>) -> ToolResult {
    let body = json!({"addLabelIds": add, "removeLabelIds": remove});
    api.send(Method::POST, &format!("{BASE}/threads/{}/modify", segment(id)), &[], Some(body)).await.map_err(|e| e.describe())?;
    Ok(format!("Done on thread {id}."))
}

async fn send_new(api: &Api, args: &Args, draft: bool) -> ToolResult {
    let to = args.list("to");
    if to.is_empty() && !draft {
        return Err("`to` is missing".into());
    }
    let mail = Mail {
        to,
        cc: args.list("cc"),
        bcc: args.list("bcc"),
        subject: args.str("subject").unwrap_or_default().to_string(),
        body: args.need("body")?.to_string(),
        in_reply_to: None,
        references: None,
    };
    deliver(api, &mail, None, draft).await
}

async fn reply(api: &Api, args: &Args) -> ToolResult {
    let thread_id = args.need("thread_id")?;
    let wanted = ["From", "To", "Cc", "Reply-To", "Subject", "Message-ID", "References"].map(|h| ("metadataHeaders", h.to_string()));
    let mut query = vec![("format", "metadata".to_string())];
    query.extend(wanted);
    let thread = api.get(&format!("{BASE}/threads/{}", segment(thread_id)), &query).await.map_err(|e| e.describe())?;
    let last = thread["messages"].as_array().and_then(|m| m.last()).cloned().ok_or("the thread has no messages")?;
    let me = api.get(&format!("{BASE}/profile"), &[]).await.map_err(|e| e.describe())?["emailAddress"].as_str().unwrap_or_default().to_lowercase();

    // Replying to one's own last message goes to its recipients.
    let from = header(&last, "From");
    let mine = address_of(&from) == me;
    let mut to = if mine { split_addresses(&header(&last, "To")) } else { split_addresses(&first_of(&header(&last, "Reply-To"), &from)) };
    let mut cc = Vec::new();
    if args.bool("reply_all").unwrap_or(false) {
        cc = split_addresses(&header(&last, "To")).into_iter().chain(split_addresses(&header(&last, "Cc"))).collect();
    }
    let not_me = |a: &String| address_of(a) != me;
    to.retain(not_me);
    cc.retain(|a| not_me(a) && !to.iter().any(|t| address_of(t) == address_of(a)));
    if to.is_empty() {
        return Err("nobody to reply to in the last message".into());
    }
    let subject = header(&last, "Subject");
    let subject = if subject.to_lowercase().starts_with("re:") { subject } else { format!("Re: {subject}") };
    let message_id = header(&last, "Message-ID");
    let references = [header(&last, "References"), message_id.clone()].iter().filter(|s| !s.is_empty()).cloned().collect::<Vec<_>>().join(" ");
    let mail = Mail {
        to,
        cc,
        bcc: Vec::new(),
        subject,
        body: args.need("body")?.to_string(),
        in_reply_to: (!message_id.is_empty()).then_some(message_id),
        references: (!references.is_empty()).then_some(references),
    };
    deliver(api, &mail, Some(thread_id), args.bool("draft").unwrap_or(false)).await
}

async fn deliver(api: &Api, mail: &Mail, thread_id: Option<&str>, draft: bool) -> ToolResult {
    let mut message = json!({"raw": URL_SAFE_NO_PAD.encode(mail.rfc5322())});
    if let Some(id) = thread_id {
        message["threadId"] = json!(id);
    }
    if draft {
        let made = api.send(Method::POST, &format!("{BASE}/drafts"), &[], Some(json!({"message": message}))).await.map_err(|e| e.describe())?;
        Ok(format!("Draft saved (id {}), to {}.", made["id"].as_str().unwrap_or("?"), mail.to.join(", ")))
    } else {
        let sent = api.send(Method::POST, &format!("{BASE}/messages/send"), &[], Some(message)).await.map_err(|e| e.describe())?;
        Ok(format!("Sent to {} (message {}, thread {}).", mail.to.join(", "), sent["id"].as_str().unwrap_or("?"), sent["threadId"].as_str().unwrap_or("?")))
    }
}

/// A plain-text message to send.
struct Mail {
    to: Vec<String>,
    cc: Vec<String>,
    bcc: Vec<String>,
    subject: String,
    body: String,
    in_reply_to: Option<String>,
    references: Option<String>,
}

impl Mail {
    /// The message as RFC 5322 bytes; the text in base64 so any UTF-8 goes.
    fn rfc5322(&self) -> Vec<u8> {
        let mut head = Vec::new();
        let mut add = |name: &str, value: &str| {
            if !value.is_empty() {
                head.push(format!("{name}: {}", one_line(value)));
            }
        };
        add("To", &self.to.join(", "));
        add("Cc", &self.cc.join(", "));
        add("Bcc", &self.bcc.join(", "));
        add("Subject", &encode_word(&self.subject));
        add("In-Reply-To", self.in_reply_to.as_deref().unwrap_or_default());
        add("References", self.references.as_deref().unwrap_or_default());
        add("MIME-Version", "1.0");
        add("Content-Type", "text/plain; charset=\"UTF-8\"");
        add("Content-Transfer-Encoding", "base64");
        let body = STANDARD.encode(self.body.replace("\r\n", "\n").replace('\n', "\r\n"));
        let wrapped: Vec<&str> = body.as_bytes().chunks(76).map(|c| std::str::from_utf8(c).expect("base64 is ASCII")).collect();
        format!("{}\r\n\r\n{}\r\n", head.join("\r\n"), wrapped.join("\r\n")).into_bytes()
    }
}

/// No line breaks in a header: they would start another one.
fn one_line(s: &str) -> String {
    s.replace(['\r', '\n'], " ")
}

/// An RFC 2047 encoded word for a header that isn't plain ASCII.
fn encode_word(s: &str) -> String {
    if s.is_ascii() { s.to_string() } else { format!("=?UTF-8?B?{}?=", STANDARD.encode(s)) }
}

fn header(message: &Value, name: &str) -> String {
    message["payload"]["headers"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|h| h["name"].as_str().is_some_and(|n| n.eq_ignore_ascii_case(name)))
        .and_then(|h| h["value"].as_str())
        .unwrap_or_default()
        .to_string()
}

fn has_label(message: &Value, label: &str) -> bool {
    message["labelIds"].as_array().is_some_and(|l| l.iter().any(|x| x == label))
}

fn first_of(a: &str, b: &str) -> String {
    if a.trim().is_empty() { b.to_string() } else { a.to_string() }
}

/// `Name <a@b.it>, c@d.it` into its addresses, names kept; commas inside
/// quotes don't split.
fn split_addresses(list: &str) -> Vec<String> {
    let (mut out, mut current, mut quoted) = (Vec::new(), String::new(), false);
    for c in list.chars() {
        match c {
            '"' => {
                quoted = !quoted;
                current.push(c);
            }
            ',' if !quoted => out.push(std::mem::take(&mut current)),
            _ => current.push(c),
        }
    }
    out.push(current);
    out.into_iter().map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect()
}

/// The bare address of `Name <a@b.it>`, lowercase.
fn address_of(s: &str) -> String {
    let inner = match (s.rfind('<'), s.rfind('>')) {
        (Some(a), Some(b)) if a < b => &s[a + 1..b],
        _ => s,
    };
    inner.trim().to_lowercase()
}

/// The readable text of a message: its text/plain part, or its HTML made plain.
fn message_text(payload: &Value) -> String {
    if let Some(text) = find_part(payload, "text/plain") {
        return text;
    }
    find_part(payload, "text/html").map(|h| html_to_text(&h)).unwrap_or_default()
}

fn find_part(part: &Value, mime: &str) -> Option<String> {
    let is_attachment = part["filename"].as_str().is_some_and(|f| !f.is_empty());
    if part["mimeType"].as_str() == Some(mime) && !is_attachment
        && let Some(data) = part["body"]["data"].as_str()
    {
        return URL_SAFE_NO_PAD_INDIFFERENT.decode(data).ok().map(|b| String::from_utf8_lossy(&b).into_owned());
    }
    part["parts"].as_array()?.iter().find_map(|p| find_part(p, mime))
}

fn collect_attachments(part: &Value, out: &mut Vec<String>) {
    if let Some(name) = part["filename"].as_str().filter(|f| !f.is_empty()) {
        out.push(name.to_string());
    }
    for p in part["parts"].as_array().into_iter().flatten() {
        collect_attachments(p, out);
    }
}

/// Crude, but enough for a model to read: no styles or scripts, no tags,
/// breaks where blocks end.
fn html_to_text(html: &str) -> String {
    let mut text = String::with_capacity(html.len() / 2);
    // ASCII only, so its byte offsets are the same as the HTML's.
    let lower = html.to_ascii_lowercase();
    let mut i = 0;
    while i < html.len() {
        if lower[i..].starts_with("<style") || lower[i..].starts_with("<script") {
            let close = if lower[i..].starts_with("<style") { "</style>" } else { "</script>" };
            i = lower[i..].find(close).map(|j| i + j + close.len()).unwrap_or(html.len());
            continue;
        }
        if html[i..].starts_with('<') {
            let end = html[i..].find('>').map(|j| i + j + 1).unwrap_or(html.len());
            let tag = &lower[i..end];
            if ["<br", "</p", "</div", "</tr", "</li", "</h"].iter().any(|t| tag.starts_with(t)) {
                text.push('\n');
            }
            i = end;
            continue;
        }
        let c = html[i..].chars().next().expect("inside the string");
        text.push(c);
        i += c.len_utf8();
    }
    let text = html_unescape(&text);
    // Collapse the blank space HTML leaves.
    let lines: Vec<String> = text.lines().map(|l| l.split_whitespace().collect::<Vec<_>>().join(" ")).collect();
    let mut out = String::new();
    let mut blank = false;
    for line in lines {
        if line.is_empty() {
            blank = !out.is_empty();
            continue;
        }
        if blank {
            out.push('\n');
            blank = false;
        }
        out.push_str(&line);
        out.push('\n');
    }
    out.trim_end().to_string()
}

fn html_unescape(s: &str) -> String {
    s.replace("&nbsp;", " ").replace("&#39;", "'").replace("&quot;", "\"").replace("&lt;", "<").replace("&gt;", ">").replace("&amp;", "&")
}

fn truncate(s: &str, max: usize) -> String {
    match s.char_indices().nth(max) {
        Some((cut, _)) => format!("{}… [cut, {} characters in all]", &s[..cut], s.chars().count()),
        None => s.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_message_is_valid_rfc5322() {
        let mail = Mail {
            to: vec!["Marco <marco@example.com>".into()],
            cc: vec![],
            bcc: vec![],
            subject: "Cena di venerdì".into(),
            body: "Ciao,\nci sei?".into(),
            in_reply_to: Some("<a@b>".into()),
            references: Some("<a@b>".into()),
        };
        let raw = String::from_utf8(mail.rfc5322()).unwrap();
        let (head, body) = raw.split_once("\r\n\r\n").unwrap();
        assert!(head.contains("To: Marco <marco@example.com>\r\n"));
        assert!(head.contains(&format!("Subject: =?UTF-8?B?{}?=", STANDARD.encode("Cena di venerdì"))));
        assert!(head.contains("In-Reply-To: <a@b>") && !head.contains("Cc:"));
        assert_eq!(STANDARD.decode(body.trim()).unwrap(), b"Ciao,\r\nci sei?");
    }

    #[test]
    fn headers_cannot_be_injected() {
        let mail = Mail { to: vec!["a@b.it\r\nBcc: evil@x.it".into()], cc: vec![], bcc: vec![], subject: "x".into(), body: "y".into(), in_reply_to: None, references: None };
        let raw = String::from_utf8(mail.rfc5322()).unwrap();
        assert!(!raw.contains("\r\nBcc:"), "{raw}");
    }

    #[test]
    fn addresses() {
        assert_eq!(split_addresses(r#""Rossi, Mario" <m@r.it>, b@c.it"#), vec![r#""Rossi, Mario" <m@r.it>"#, "b@c.it"]);
        assert_eq!(address_of("Mario <M@R.it>"), "m@r.it");
        assert_eq!(address_of("m@r.it"), "m@r.it");
    }

    #[test]
    fn the_text_part_wins_and_html_is_made_plain() {
        let enc = |s: &str| URL_SAFE_NO_PAD.encode(s);
        let payload = json!({"mimeType": "multipart/alternative", "parts": [
            {"mimeType": "text/html", "body": {"data": enc("<p>Hi</p>")}},
            {"mimeType": "text/plain", "body": {"data": enc("Hi there")}},
            {"mimeType": "application/pdf", "filename": "a.pdf", "body": {"attachmentId": "x"}}
        ]});
        assert_eq!(message_text(&payload), "Hi there");
        let mut names = vec![];
        collect_attachments(&payload, &mut names);
        assert_eq!(names, vec!["a.pdf"]);
        let html = "<html><style>p{color:red}</style><p>Ciao&nbsp;Marco,</p><p>a   presto</p><br>&amp; grazie</html>";
        assert_eq!(html_to_text(html), "Ciao Marco,\na presto\n\n& grazie");
    }

    #[test]
    fn long_text_is_cut() {
        assert_eq!(truncate("abc", 5), "abc");
        assert!(truncate("abcdef", 3).starts_with("abc… [cut, 6"));
    }

    #[test]
    fn a_thread_summary() {
        let thread = json!({"id": "t1", "messages": [
            {"labelIds": ["INBOX"], "snippet": "first", "payload": {"headers": [{"name": "Subject", "value": "Hello"}, {"name": "From", "value": "a@x.it"}]}},
            {"labelIds": ["UNREAD"], "snippet": "it&#39;s me", "payload": {"headers": [{"name": "From", "value": "b@x.it"}, {"name": "Date", "value": "today"}]}}
        ]});
        let s = thread_summary(&thread);
        assert_eq!(s["subject"], "Hello");
        assert_eq!(s["from"], "b@x.it");
        assert_eq!(s["unread"], true);
        assert_eq!(s["snippet"], "it's me");
    }
}
