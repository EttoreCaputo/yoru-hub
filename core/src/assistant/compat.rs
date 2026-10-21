//! The assistant on a brain that speaks OpenAI's chat completions and
//! function calling ([`cloud::Client::chat`]): OpenAI ([`Brain::OpenAi`]),
//! Groq ([`Brain::Groq`]: free, with daily limits) and Ollama
//! ([`Brain::Ollama`]: a model on this computer).
//!
//! What differs from Claude, and why:
//! - **No tool search: the model finds and loads the tools.** A turn starts
//!   with Yoru's own tools, the board's essentials (`react`, `get_state`),
//!   and the tools the conversation already used; the prompt only names the
//!   connected services and the board. The model searches their tools with `find_tools` (names and a
//!   line each, [`select`]) and loads the ones it wants with `load_tools`;
//!   the board's settings (volume, brightness, the menu…) are found the same way.
//!   So many services cost a line each, not every schema on every request.
//!   How many tools stay loaded, and how much of the memory, the history and
//!   the tools' results, is the brain's [`Budget`]: Groq's free plan allows
//!   about 8,000 tokens a minute, reserved answer included, and a model on
//!   this computer reads a short prompt best.
//! - **Web search only through Tavily.** OpenAI searches only through its
//!   Responses API, and Groq's models that search (compound) aren't used:
//!   without the Tavily connector the model is told it can't.
//! - **Limits are said plainly.** A 429 on Groq becomes "the free limit for
//!   the day (or the minute) was reached, try again in…" ([`brain_error`]);
//!   the model isn't asked again until then ([`cloud::Client::limit`]).

use std::time::Instant;

use serde_json::{Value, json};

use super::{Action, Assistant, BOARD_ESSENTIALS, Brain, ExternalTool, Host, MAX_ROUNDS, Reply, Settings, board_extras, own_tools, schema_or_empty, select, tool_json};
use crate::cloud::{self, CloudError, Client};
use crate::models::Kind;
use crate::timers::Timers;

/// How much a request carries, by brain.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Budget {
    /// Tokens of an answer: room for its reasoning, a tool call or a long
    /// reply. Groq counts them against the minute's tokens before answering.
    pub max_tokens: u32,
    /// The same, for a triage's answer: a list of items.
    pub json_max_tokens: u32,
    /// Connector tools loaded at once; loading more lets the oldest go.
    pub loaded: usize,
    /// Tools a `find_tools` answer lists.
    pub found: usize,
    /// The memory it reads, in characters.
    pub memory_max: usize,
    /// A tool's result, in characters: a mailbox's search can be 60 KB.
    pub result_max: usize,
    /// The same, for the turns before this one.
    pub old_result_max: usize,
    /// A tool's description, in characters.
    pub description_max: usize,
    /// The user's turns kept from the conversation.
    pub turns_kept: usize,
}

/// Groq's free plan and a model on this computer: every token counts.
const TIGHT: Budget = Budget { max_tokens: 1024, json_max_tokens: 2048, loaded: 6, found: 8, memory_max: 3000, result_max: 6000, old_result_max: 600, description_max: 300, turns_kept: 4 };
/// OpenAI: paid by the token, with a long context.
const ROOMY: Budget = Budget { max_tokens: 4096, json_max_tokens: 4096, loaded: 16, found: 12, memory_max: 12_000, result_max: 30_000, old_result_max: 2000, description_max: 1000, turns_kept: 10 };

pub fn budget(brain: Brain) -> Budget {
    if brain == Brain::OpenAi { ROOMY } else { TIGHT }
}

/// The settings' model of `brain`, with its key.
pub fn client(settings: &Settings, brain: Brain) -> Result<Client, String> {
    let model = settings.model_of(brain);
    if model.is_empty() {
        return Err(format!("no {} model chosen: pick one in Settings", brain.name()));
    }
    Client::new(&format!("{}:{model}", brain.provider().id), Kind::Llm)
}

/// The chat models of `brain`'s catalog: their ids (`openai/gpt-oss-120b`)
/// and notes. Ollama has none: its models are the person's ([`cloud::list_models`]).
pub fn models(brain: Brain) -> Vec<(&'static str, &'static str)> {
    brain.provider().models.iter().filter(|m| m.kind == Kind::Llm).map(|m| (m.id, m.note)).collect()
}

/// Whether `model` follows a JSON schema strictly (structured outputs:
/// OpenAI's, Groq's GPT-OSS and Qwen, October 2026); the others get JSON mode.
fn follows_schema(brain: Brain, model: &str) -> bool {
    match brain {
        Brain::OpenAi => true,
        Brain::Groq => model.starts_with("openai/gpt-oss") || model == "qwen/qwen3.8-27b",
        Brain::Anthropic | Brain::Ollama => false,
    }
}

/// What the user reads when the brain failed.
pub fn brain_error(e: &CloudError) -> String {
    match e {
        CloudError::RateLimited { provider: "Groq", retry_after, per_day } => {
            let when = retry_after.map(|d| format!(": try again in {}", cloud::wait_text(d))).unwrap_or_else(|| ": try again in a minute".into());
            let what = if *per_day { "Groq's free limit for the day was reached" } else { "Groq's free limit for the minute was reached" };
            let hint = if *per_day { " (or switch Yoru's brain in Settings)" } else { "" };
            format!("{what}{when}{hint}")
        }
        CloudError::Status { provider: "Groq", status: 413, .. } => "the request was too big for Groq's free plan: start a new conversation, or ask about one service at a time".into(),
        CloudError::Network { provider: "Ollama", .. } => "Ollama doesn't answer: is it running on this computer?".into(),
        CloudError::Status { provider: "Ollama", status: 404, .. } => "Ollama doesn't have that model: pull it, or pick another in Settings".into(),
        CloudError::Status { provider: "Ollama", status: 400, message } if message.contains("does not support tools") => {
            "that Ollama model can't use tools: pick one that can (qwen3, llama3.1, mistral…)".into()
        }
        e => e.to_string(),
    }
}

/// Who Yoru is, in fewer words than Claude's prompt: on Groq every token
/// counts against the free plan's minute.
fn system_prompt(name: &str, services: &str, web: bool) -> String {
    let services = if services.is_empty() {
        "No services are connected yet.".to_string()
    } else {
        format!(
            "Beyond your own tools you can reach: {services}. Their tools aren't loaded until you need them: find them with find_tools, \
then load the ones you'll call with load_tools; the ones already loaded are in your tools."
        )
    };
    let web = if web {
        "You can search the web, news included, with Tavily's search: find and load it like the other tools."
    } else {
        "You can't search the web: if a question needs it, say so."
    };
    format!(
        "You are {name}, the user's personal assistant. Your body is a small round screen on their desk (a sphere with two eyes), with a microphone and a speaker. \
{services} {web}
- Use the tools instead of guessing; never invent facts about the user's data.
- Tools that change something ask the user first; if they say no, accept it.
- Too long to say aloud? Say the gist and put the details in the inbox with notify (urgency: low inbox only, normal banner, high knock, urgent said aloud; be sparing).
- Save lasting facts about the user with remember, remove stale ones with forget. The memory is in <memory>.
- Reply in the user's language. Channel voice: one to three plain sentences, no markdown, lists, emoji or URLs. Channel text: short Markdown is fine.
- Friendly and to the point; never chatty or servile; don't end with offers of help."
    )
}

/// Whether a connected service searches the web: Tavily's search is the one
/// these brains have, since they get no server tool for it.
fn searches_web(tools: &[ExternalTool]) -> bool {
    tools.iter().any(|t| t.name == "tavily__tavily_search")
}

/// The connected services for the prompt, with how many tools each has:
/// "Gmail (12 tools), Tavily (4 tools)".
fn services_line(tools: &[ExternalTool]) -> String {
    let mut services: Vec<(&str, usize)> = Vec::new();
    for t in tools {
        match services.iter_mut().find(|(s, _)| *s == t.service) {
            Some((_, n)) => *n += 1,
            None => services.push((&t.service, 1)),
        }
    }
    services.iter().map(|(s, n)| format!("{s} ({n} tool{})", if *n == 1 { "" } else { "s" })).collect::<Vec<_>>().join(", ")
}

/// `find_tools`: what the services' tools do, without loading any.
fn find_tools_tool() -> Value {
    json!({
        "name": "find_tools",
        "description": "Search the tools of the user's services and of your board by what they do (e.g. \"unread mail\", \"events tomorrow\", \
\"web search\", \"volume\"). Answers each tool's name and what it does; load the ones you need with load_tools.",
        "input_schema": {"type": "object", "properties": {"query": {"type": "string"}}, "required": ["query"]},
    })
}

/// `load_tools`: the tools `find_tools` named, callable from the next step.
fn load_tools_tool(budget: &Budget) -> Value {
    json!({
        "name": "load_tools",
        "description": format!("Load tools named by find_tools so you can call them; at most {} stay loaded, the oldest go first.", budget.loaded),
        "input_schema": {"type": "object", "properties": {"names": {"type": "array", "items": {"type": "string"}}}, "required": ["names"]},
    })
}

/// Anthropic's tool shape to OpenAI's function.
fn function(tool: &Value, description_max: usize) -> Value {
    let description: String = tool["description"].as_str().unwrap_or_default().chars().take(description_max).collect();
    let mut parameters = schema_or_empty(&tool["input_schema"]);
    if let Some(map) = parameters.as_object_mut() {
        map.remove("$schema");
    }
    json!({"type": "function", "function": {"name": tool["name"], "description": description, "parameters": parameters}})
}

fn external_json(t: &ExternalTool) -> Value {
    let mut description = format!("[{}] {}", t.service, t.description);
    if t.acts {
        description.push_str(" (Asks the user first.)");
    }
    json!({"name": t.name, "description": description, "input_schema": t.input_schema})
}

/// What the model may call: Yoru's tools, the timers, the board's
/// essentials, the two that reach the other tools when there are any, and
/// those loaded.
fn tool_list(board: &[crate::link::Tool], loaded: &[&ExternalTool], findable: bool, budget: &Budget) -> Vec<Value> {
    let mut tools = own_tools();
    tools.extend(Timers::tools().iter().map(tool_json));
    tools.extend(board.iter().filter(|t| BOARD_ESSENTIALS.contains(&t.name.as_str())).map(tool_json));
    if findable {
        tools.push(find_tools_tool());
        tools.push(load_tools_tool(budget));
    }
    tools.extend(loaded.iter().map(|t| external_json(t)));
    tools.iter().map(|t| function(t, budget.description_max)).collect()
}

/// Keep the last [`Budget::turns_kept`] turns, starting at a user's
/// message, and shrink the tool results before them.
fn trim(messages: &mut Vec<Value>, budget: &Budget) {
    let starts: Vec<usize> = messages.iter().enumerate().filter(|(_, m)| m["role"] == "user").map(|(i, _)| i).collect();
    if starts.len() > budget.turns_kept {
        messages.drain(..starts[starts.len() - budget.turns_kept]);
    }
    for m in messages.iter_mut().filter(|m| m["role"] == "tool") {
        if let Some(text) = m["content"].as_str().filter(|t| t.chars().count() > budget.old_result_max) {
            m["content"] = Value::String(cut(text, budget.old_result_max));
        }
    }
}

fn cut(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    format!("{}\n[… cut]", text.chars().take(max).collect::<String>())
}

/// The tools the conversation called already.
fn used_tools(messages: &[Value]) -> Vec<String> {
    let mut used = Vec::new();
    for call in messages.iter().filter_map(|m| m["tool_calls"].as_array()).flatten() {
        if let Some(name) = call.pointer("/function/name").and_then(Value::as_str)
            && !used.iter().any(|u| u == name)
        {
            used.push(name.to_string());
        }
    }
    used
}

/// A thinking model's text without its thoughts, when it leaves them in.
fn clean(text: &str) -> String {
    let text = match (text.find("<think>"), text.find("</think>")) {
        (Some(start), Some(end)) if end > start => format!("{}{}", &text[..start], &text[end + "</think>".len()..]),
        _ => text.to_string(),
    };
    text.trim().to_string()
}

/// One request. A tool call the model wrote badly (Groq's 400
/// `tool_use_failed`) is asked again once.
fn chat(client: &Client, request: &Value) -> Result<Value, String> {
    match client.chat(request) {
        Err(CloudError::Status { status: 400, message, .. }) if message.contains("tool") && message.contains("failed") => client.chat(request).map_err(|e| brain_error(&e)),
        reply => reply.map_err(|e| brain_error(&e)),
    }
}

/// A turn on `brain`: `prompt` after `messages` (OpenAI's form), the tools
/// run until the model answers in words.
pub(super) fn run(a: &Assistant, brain: Brain, messages: &mut Vec<Value>, prompt: &str, settings: &Settings, host: &dyn Host) -> Result<Reply, String> {
    let client = client(settings, brain)?;
    let budget = budget(brain);
    let started = Instant::now();
    trim(messages, &budget);
    let used = used_tools(messages);
    messages.push(json!({"role": "user", "content": prompt}));
    let external_tools = a.external().map(|e| e.tools()).unwrap_or_default();
    let board_tools = host.board_tools();
    // What find_tools searches: the services' tools and the board's beyond its essentials.
    let findable: Vec<ExternalTool> = external_tools.iter().cloned().chain(board_extras(&board_tools)).collect();
    // A follow-up ("and reply to it") still has the tools the conversation used.
    let mut loaded: Vec<&ExternalTool> = used.iter().filter_map(|n| findable.iter().find(|t| &t.name == n)).collect();
    loaded.drain(..loaded.len().saturating_sub(budget.loaded));
    let system = json!({"role": "system", "content": format!(
        "{}\n\n<memory>\n{}\n</memory>",
        system_prompt(&settings.name, &services_line(&findable), searches_web(&external_tools)),
        a.memory.for_prompt_within(budget.memory_max)
    )});
    let model = format!("{}:{}", brain.provider().id, settings.model_of(brain));
    let mut actions: Vec<Action> = Vec::new();
    for _ in 0..MAX_ROUNDS {
        let tools = tool_list(&board_tools, &loaded, !findable.is_empty(), &budget);
        let mut all = vec![system.clone()];
        all.extend(messages.iter().cloned());
        let message = chat(&client, &json!({"messages": all, "tools": tools, "tool_choice": "auto", "max_tokens": budget.max_tokens}))?;
        let calls = message["tool_calls"].as_array().cloned().unwrap_or_default();
        let text = clean(message["content"].as_str().unwrap_or_default());
        if calls.is_empty() {
            messages.push(json!({"role": "assistant", "content": text}));
            return Ok(Reply { text, actions, model, elapsed_ms: started.elapsed().as_millis() as u64 });
        }
        // Only what the API takes back: no reasoning, no extra fields.
        messages.push(json!({"role": "assistant", "content": if text.is_empty() { Value::Null } else { Value::String(text) }, "tool_calls": calls}));
        for call in &calls {
            let id = call["id"].as_str().unwrap_or_default();
            let name = call.pointer("/function/name").and_then(Value::as_str).unwrap_or_default();
            let arguments = call.pointer("/function/arguments").and_then(Value::as_str).unwrap_or("{}");
            let input = serde_json::from_str::<Value>(if arguments.trim().is_empty() { "{}" } else { arguments }).map_err(|e| format!("error: the arguments weren't valid JSON ({e})"));
            let text = match name {
                "find_tools" => found(&findable, &loaded, input.as_ref().ok().and_then(|i| i["query"].as_str()).unwrap_or_default(), budget.found),
                "load_tools" => {
                    let names: Vec<&str> = input.as_ref().ok().and_then(|i| i["names"].as_array()).map(|a| a.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
                    load(&findable, &mut loaded, &names, budget.loaded)
                }
                _ => {
                    let (text, ok) = a.run_one(name, input.as_ref().map_err(Clone::clone), &board_tools, &external_tools, host, &mut actions);
                    if ok { text } else { format!("error: {text}") }
                }
            };
            messages.push(json!({"role": "tool", "tool_call_id": id, "content": cut(&text, budget.result_max)}));
        }
    }
    Err("it took too many steps; ask again more simply".into())
}

/// A found tool's line: its name and the start of what it does.
const FOUND_LINE_MAX: usize = 140;

/// `find_tools`: the best `count` tools for `query` that aren't loaded, a
/// line each. A query that names none ("anything new?") gets the tools that
/// read, a few per service.
fn found(all: &[ExternalTool], loaded: &[&ExternalTool], query: &str, count: usize) -> String {
    let rest: Vec<ExternalTool> = all.iter().filter(|t| !loaded.iter().any(|l| l.name == t.name)).cloned().collect();
    let mut hits = select::find(&rest, query, count);
    let head = if hits.is_empty() {
        hits = select::readers(&rest, count);
        if hits.is_empty() {
            return "no other tool; answer with what you have".into();
        }
        "Nothing matches; these read:"
    } else {
        "Found:"
    };
    let lines: Vec<String> = hits
        .iter()
        .map(|t| {
            let what: String = t.description.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(FOUND_LINE_MAX).collect();
            format!("- {}: {what}{}", t.name, if t.acts { " (asks the user first)" } else { "" })
        })
        .collect();
    format!("{head}\n{}\nLoad the ones you need with load_tools.", lines.join("\n"))
}

/// `load_tools`: `names` join `loaded`, the oldest leaving past `max`.
fn load<'a>(all: &'a [ExternalTool], loaded: &mut Vec<&'a ExternalTool>, names: &[&str], max: usize) -> String {
    let mut added = Vec::new();
    let mut unknown = Vec::new();
    for name in names.iter().take(max) {
        match all.iter().find(|t| t.name == *name) {
            Some(t) if !loaded.iter().any(|l| l.name == t.name) => {
                loaded.push(t);
                added.push(*name);
            }
            Some(_) => {}
            None => unknown.push(*name),
        }
    }
    let gone: Vec<String> = loaded.drain(..loaded.len().saturating_sub(max)).map(|t| t.name.clone()).collect();
    let mut text = if added.is_empty() { "nothing new loaded".to_string() } else { format!("loaded: {}", added.join(", ")) };
    if !gone.is_empty() {
        text.push_str(&format!("; unloaded to make room: {}", gone.join(", ")));
    }
    if !unknown.is_empty() {
        text.push_str(&format!("; no tool named {} (use the names find_tools gives)", unknown.join(", ")));
    }
    text
}

/// One answer in JSON for `schema`: strict structured output where the
/// model has it, else JSON mode with the schema in the prompt.
pub(super) fn complete_json(settings: &Settings, brain: Brain, prompt: &str, schema: &Value) -> Result<String, String> {
    let client = client(settings, brain)?;
    let (prompt, format) = if follows_schema(brain, settings.model_of(brain)) {
        (prompt.to_string(), json!({"type": "json_schema", "json_schema": {"name": "answer", "strict": true, "schema": schema}}))
    } else {
        (format!("{prompt}\n\nAnswer with JSON only, matching this schema:\n{schema}"), json!({"type": "json_object"}))
    };
    let request = json!({"messages": [{"role": "user", "content": prompt}], "response_format": format, "max_tokens": budget(brain).json_max_tokens});
    let message = client.chat(&request).map_err(|e| brain_error(&e))?;
    Ok(clean(message["content"].as_str().unwrap_or_default()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assistant::tests::{Mail, NoBoard, assistant, fake_server};
    use crate::assistant::{Brain, Channel, ExternalTools, Settings};
    use std::sync::Arc;

    /// The fake server's address for Groq, and its key, for this process.
    fn point_groq_at(url: &str) {
        // SAFETY: only the Groq tests read these variables, and they set the same ones.
        unsafe {
            std::env::set_var("GROQ_BASE_URL", url);
            std::env::set_var("GROQ_API_KEY", "test-key");
        }
    }

    fn groq_settings() -> Settings {
        Settings { provider: Some(Brain::Groq), ..Settings::default() }
    }

    fn answer(message: Value) -> (u16, Value) {
        (200, json!({"choices": [{"index": 0, "message": message, "finish_reason": "stop"}]}))
    }

    #[test]
    fn a_turn_on_groq_calls_tools_and_answers() {
        let _env = crate::assistant::tests::ENV.lock().unwrap_or_else(|e| e.into_inner());
        let (url, requests) = fake_server(vec![
            answer(json!({"role": "assistant", "content": null, "reasoning": "hidden", "tool_calls": [
                {"id": "c1", "type": "function", "function": {"name": "remember", "arguments": "{\"section\": \"about\", \"text\": \"Likes espresso\"}"}},
                {"id": "c2", "type": "function", "function": {"name": "gmail__send", "arguments": "{\"to\": \"marco@example.com\"}"}},
            ]})),
            answer(json!({"role": "assistant", "content": "Fatto, me lo ricordo."})),
        ]);
        point_groq_at(&url);
        let a = assistant();
        a.set_settings(groq_settings()).unwrap();
        a.set_external(Some(Arc::new(Mail)));
        let reply = a.respond("Ricordati che amo l'espresso e manda una mail a Marco", Some("it"), Channel::Voice, &NoBoard).unwrap();
        assert_eq!(reply.text, "Fatto, me lo ricordo.");
        assert_eq!(reply.model, "groq:openai/gpt-oss-120b");
        assert!(a.memory.text().contains("- Likes espresso"));
        // The send asked first, and NoBoard said no.
        assert_eq!(reply.actions.len(), 2);
        assert!(!reply.actions[1].ok);
        let first = requests.recv().unwrap();
        assert_eq!(first["model"], "openai/gpt-oss-120b");
        assert!(first["max_tokens"].as_u64().unwrap() >= u64::from(TIGHT.max_tokens));
        assert_eq!(first["messages"][0]["role"], "system");
        assert!(first["messages"][1]["content"].as_str().unwrap().contains("channel: voice, language: it"));
        let names: Vec<&str> = first["tools"].as_array().unwrap().iter().filter_map(|t| t.pointer("/function/name").and_then(Value::as_str)).collect();
        // The services are named; none of their tools is loaded yet.
        assert!(names.contains(&"notify") && names.contains(&"find_tools") && names.contains(&"load_tools"));
        assert!(!names.contains(&"gmail__send") && !names.contains(&"web_search"));
        assert!(first["messages"][0]["content"].as_str().unwrap().contains("you can reach: Gmail (2 tools)."));
        // The second carries the calls back without the reasoning, then the results.
        let second = requests.recv().unwrap();
        let back = &second["messages"][2];
        assert_eq!(back["tool_calls"][0]["id"], "c1");
        assert!(back.get("reasoning").is_none());
        assert_eq!(second["messages"][3]["role"], "tool");
        assert_eq!(second["messages"][3]["tool_call_id"], "c1");
        assert!(second["messages"][4]["content"].as_str().unwrap().starts_with("error: the user said no"));
    }

    #[test]
    fn a_limit_on_groq_is_said_plainly_and_not_asked_again() {
        let _env = crate::assistant::tests::ENV.lock().unwrap_or_else(|e| e.into_inner());
        let (url, requests) = fake_server(vec![(
            429,
            json!({"error": {"message": "Rate limit reached for model `qwen/qwen3.8-27b` on tokens per day (TPD): Limit 100000, Used 99990. Please try again in 12m5s.", "code": "rate_limit_exceeded"}}),
        )]);
        point_groq_at(&url);
        let a = assistant();
        // A model of its own, so the limit doesn't reach the other tests.
        a.set_settings(Settings { groq_model: "qwen/qwen3.8-27b".into(), ..groq_settings() }).unwrap();
        let e = a.respond("Che mail ho?", Some("it"), Channel::Text, &NoBoard).unwrap_err();
        assert!(e.contains("limit for the day") && e.contains("13 min"), "{e}");
        assert!(a.chat().last().unwrap().text.contains("limit for the day"));
        let limit = a.limited().expect("remembered");
        assert!(super::super::limit_line(&limit, "it").starts_with("Ho finito i pensieri gratuiti di oggi. Riprova tra 13 minuti"));
        assert!(super::super::limit_line(&limit, "en").contains("13 minutes"));
        // Asked again at once: no request goes out (the server answers only one).
        let again = a.respond("E adesso?", Some("it"), Channel::Text, &NoBoard).unwrap_err();
        assert!(again.contains("limit for the day"));
        requests.recv().unwrap();
        assert!(requests.try_recv().is_err());
    }

    #[test]
    fn triage_on_groq_uses_structured_output() {
        let _env = crate::assistant::tests::ENV.lock().unwrap_or_else(|e| e.into_inner());
        let items = r#"{"items": [{"key": "m1", "notify": true, "urgency": "normal", "title": "Marco", "text": "Needs you", "url": ""}]}"#;
        let (url, requests) = fake_server(vec![answer(json!({"role": "assistant", "content": items})), answer(json!({"role": "assistant", "content": items}))]);
        point_groq_at(&url);
        let a = assistant();
        a.set_settings(Settings { groq_model: "openai/gpt-oss-20b".into(), ..groq_settings() }).unwrap();
        let schema = json!({"type": "object"});
        assert_eq!(a.complete_json("list them", &schema).unwrap(), items);
        let first = requests.recv().unwrap();
        assert_eq!(first["response_format"]["type"], "json_schema");
        assert_eq!(first["response_format"]["json_schema"]["strict"], true);
        // Llama has JSON mode only: the schema goes in the prompt.
        a.set_settings(Settings { groq_model: "llama-3.3-70b-versatile".into(), ..groq_settings() }).unwrap();
        a.complete_json("list them", &schema).unwrap();
        let second = requests.recv().unwrap();
        assert_eq!(second["response_format"]["type"], "json_object");
        assert!(second["messages"][0]["content"].as_str().unwrap().contains("matching this schema"));
    }

    #[test]
    fn history_is_trimmed_and_tools_remembered() {
        let mut messages = Vec::new();
        for i in 0..6 {
            messages.push(json!({"role": "user", "content": format!("question {i}")}));
            messages.push(json!({"role": "assistant", "content": null, "tool_calls": [{"id": "c", "type": "function", "function": {"name": format!("tool{i}"), "arguments": "{}"}}]}));
            messages.push(json!({"role": "tool", "tool_call_id": "c", "content": "x".repeat(2000)}));
            messages.push(json!({"role": "assistant", "content": "ok"}));
        }
        trim(&mut messages, &TIGHT);
        assert_eq!(messages[0]["content"], "question 2");
        assert_eq!(messages.iter().filter(|m| m["role"] == "user").count(), TIGHT.turns_kept);
        assert!(messages.iter().filter(|m| m["role"] == "tool").all(|m| m["content"].as_str().unwrap().chars().count() < 700));
        assert_eq!(used_tools(&messages), ["tool2", "tool3", "tool4", "tool5"]);
        assert_eq!(clean("<think>hmm</think> Ciao"), "Ciao");
    }

    #[test]
    fn the_model_finds_then_loads_a_tool() {
        let _env = crate::assistant::tests::ENV.lock().unwrap_or_else(|e| e.into_inner());
        let call = |id: &str, name: &str, arguments: &str| answer(json!({"role": "assistant", "content": null, "tool_calls": [
            {"id": id, "type": "function", "function": {"name": name, "arguments": arguments}},
        ]}));
        let (url, requests) = fake_server(vec![
            call("c1", "find_tools", r#"{"query": "search mail"}"#),
            call("c2", "load_tools", r#"{"names": ["gmail__search", "gmail__nope"]}"#),
            call("c3", "gmail__search", "{}"),
            answer(json!({"role": "assistant", "content": "Nessuna mail nuova."})),
        ]);
        point_groq_at(&url);
        let a = assistant();
        a.set_settings(Settings { groq_model: "openai/gpt-oss-20b".into(), ..groq_settings() }).unwrap();
        a.set_external(Some(Arc::new(Mail)));
        assert_eq!(a.respond("Ho mail nuove?", Some("it"), Channel::Text, &NoBoard).unwrap().text, "Nessuna mail nuova.");
        let names = |r: &Value| -> Vec<String> { r["tools"].as_array().unwrap().iter().filter_map(|t| t.pointer("/function/name").and_then(Value::as_str)).map(String::from).collect() };
        let first = requests.recv().unwrap();
        assert!(!names(&first).iter().any(|n| n.starts_with("gmail__")));
        let second = requests.recv().unwrap();
        let listed = second["messages"].as_array().unwrap().last().unwrap()["content"].as_str().unwrap().to_string();
        assert!(listed.contains("- gmail__search: Search mail") && listed.contains("load_tools"), "{listed}");
        assert!(!names(&second).iter().any(|n| n.starts_with("gmail__")));
        let third = requests.recv().unwrap();
        let loaded = third["messages"].as_array().unwrap().last().unwrap()["content"].as_str().unwrap().to_string();
        assert!(loaded.starts_with("loaded: gmail__search") && loaded.contains("no tool named gmail__nope"), "{loaded}");
        assert!(names(&third).contains(&"gmail__search".to_string()) && !names(&third).contains(&"gmail__send".to_string()));
    }

    /// A board with its essentials and one setting, that says what it ran.
    struct Board;
    impl Host for Board {
        fn board_tools(&self) -> Vec<crate::link::Tool> {
            ["react", "get_state", "set_volume"]
                .into_iter()
                .map(|name| crate::link::Tool { name: name.into(), description: format!("{name}: the speaker volume"), input_schema: json!({"type": "object"}) })
                .collect()
        }
        fn call_board(&self, name: &str, _: &Value) -> Result<String, String> {
            Ok(format!("board ran {name}"))
        }
        fn confirm(&self, _: &str, _: &str, _: &str) -> Option<bool> {
            Some(false)
        }
    }

    #[test]
    fn the_boards_settings_are_found_like_a_service() {
        let _env = crate::assistant::tests::ENV.lock().unwrap_or_else(|e| e.into_inner());
        let call = |id: &str, name: &str, arguments: &str| answer(json!({"role": "assistant", "content": null, "tool_calls": [
            {"id": id, "type": "function", "function": {"name": name, "arguments": arguments}},
        ]}));
        let (url, requests) = fake_server(vec![
            call("c1", "load_tools", r#"{"names": ["set_volume"]}"#),
            call("c2", "set_volume", r#"{"percent": 30}"#),
            answer(json!({"role": "assistant", "content": "Fatto."})),
        ]);
        point_groq_at(&url);
        let a = assistant();
        a.set_settings(Settings { groq_model: "openai/gpt-oss-20b".into(), ..groq_settings() }).unwrap();
        let reply = a.respond("Abbassa il volume", Some("it"), Channel::Text, &Board).unwrap();
        // Finding and loading aren't actions: the volume is the only one.
        assert_eq!(reply.actions.len(), 1);
        assert_eq!(reply.actions[0].summary, "board ran set_volume");
        let names = |r: &Value| -> Vec<String> { r["tools"].as_array().unwrap().iter().filter_map(|t| t.pointer("/function/name").and_then(Value::as_str)).map(String::from).collect() };
        let first = requests.recv().unwrap();
        assert!(names(&first).contains(&"react".to_string()) && names(&first).contains(&"get_state".to_string()));
        assert!(!names(&first).contains(&"set_volume".to_string()) && names(&first).contains(&"find_tools".to_string()));
        assert!(first["messages"][0]["content"].as_str().unwrap().contains("you can reach: Yoru's board (1 tool)."));
        assert!(names(&requests.recv().unwrap()).contains(&"set_volume".to_string()));
    }

    #[test]
    fn loading_past_the_budget_lets_the_oldest_go() {
        let all = Mail.tools();
        let mut loaded: Vec<&ExternalTool> = vec![&all[0]];
        let text = load(&all, &mut loaded, &["gmail__send"], 1);
        assert_eq!(text, "loaded: gmail__send; unloaded to make room: gmail__search");
        assert_eq!(loaded.len(), 1);
        assert_eq!(load(&all, &mut loaded, &["gmail__send"], 1), "nothing new loaded");
        // What's loaded isn't found again; a query that names nothing gets the readers.
        assert!(found(&all, &loaded, "send", 5).starts_with("Nothing matches; these read:\n- gmail__search"));
        assert!(found(&all, &[&all[0], &all[1]], "mail", 5).starts_with("no other tool"));
        assert_eq!(services_line(&all), "Gmail (2 tools)");
    }

    #[test]
    fn openai_and_ollama_take_the_same_turn() {
        let _env = crate::assistant::tests::ENV.lock().unwrap_or_else(|e| e.into_inner());
        let (url, requests) = fake_server(vec![answer(json!({"role": "assistant", "content": "Ciao!"})), answer(json!({"role": "assistant", "content": "Eccomi."}))]);
        // SAFETY: only these tests read these variables, under ENV.
        unsafe {
            std::env::set_var("OPENAI_BASE_URL", &url);
            std::env::set_var("OPENAI_API_KEY", "test-key");
            std::env::set_var("OLLAMA_BASE_URL", &url);
        }
        let a = assistant();
        a.set_settings(Settings { provider: Some(Brain::OpenAi), ..Settings::default() }).unwrap();
        assert!(a.is_ready());
        let reply = a.respond("Ciao", Some("it"), Channel::Text, &NoBoard).unwrap();
        assert_eq!(reply.model, "openai:gpt-6-luna");
        let first = requests.recv().unwrap();
        assert_eq!(first["model"], "gpt-6-luna");
        assert_eq!(first["max_completion_tokens"], u64::from(ROOMY.max_tokens));
        // Ollama needs no key, but a model the person picks.
        a.set_settings(Settings { provider: Some(Brain::Ollama), ..Settings::default() }).unwrap();
        assert!(!a.is_ready());
        assert!(a.respond("Ciao", Some("it"), Channel::Text, &NoBoard).unwrap_err().contains("no Ollama model"));
        a.set_settings(Settings { provider: Some(Brain::Ollama), ollama_model: "qwen3:8b".into(), ..Settings::default() }).unwrap();
        assert!(a.is_ready());
        assert_eq!(a.respond("Ciao", Some("it"), Channel::Text, &NoBoard).unwrap().model, "ollama:qwen3:8b");
        let second = requests.recv().unwrap();
        assert_eq!((second["model"].as_str(), second["max_tokens"].as_u64()), (Some("qwen3:8b"), Some(u64::from(TIGHT.max_tokens))));
        unsafe {
            std::env::remove_var("OPENAI_BASE_URL");
            std::env::remove_var("OPENAI_API_KEY");
            std::env::remove_var("OLLAMA_BASE_URL");
        }
    }

    #[test]
    fn errors_read_well() {
        let minute = CloudError::RateLimited { provider: "Groq", retry_after: Some(std::time::Duration::from_secs(7)), per_day: false };
        assert_eq!(brain_error(&minute), "Groq's free limit for the minute was reached: try again in 7 s");
        assert!(brain_error(&CloudError::Status { provider: "Groq", status: 413, message: String::new() }).contains("too big"));
        assert!(brain_error(&CloudError::Network { provider: "Ollama", why: "refused".into() }).contains("running"));
        let names: Vec<&str> = models(Brain::Groq).into_iter().map(|(id, _)| id).collect();
        assert!(names.contains(&super::super::DEFAULT_GROQ_MODEL) && names.contains(&"llama-3.3-70b-versatile"));
    }
}
