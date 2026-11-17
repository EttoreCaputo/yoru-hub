//! The assistant's side of Gemini Live ([`crate::cloud::live`]): what a
//! session is set up with, the tools it calls, and its turns kept in the
//! conversation and the memory like any other (docs/assistant.md, Gemini Live).
//!
//! Live hears the request itself, so what the other brains get per request
//! (the facts recalled for it, who speaks) isn't there: it
//! has the memory's core, set once per session.

use serde_json::Value;

use super::*;
use crate::cloud::live::Setup;

/// Who Yoru is, for a voice that speaks as it thinks: no audio tags,
/// which it would read out.
fn live_prompt(name: &str, notch: bool) -> String {
    let body = if notch {
        "Your body here is a small you in the Mac's notch, beside the camera: the user talks by holding a key, and you answer through the Mac's speakers."
    } else {
        "Your body is a small round screen on their desk (a sphere with two eyes), with a microphone and a speaker."
    };
    format!(
        "You are the user's personal assistant. {body} \
You talk with the user out loud.
- Use the tools instead of guessing; never invent facts about the user's data. You can search the web with Google Search.
- Never say you did something (sent, started, added, saved) unless the tool that does it answered that it did: if you haven't called it, call it.
- Tools that change something ask the user first; if they say no, accept it.
- Too long to say aloud? Say the gist and put the details in the inbox with notify (urgency: low inbox only, normal banner, high knock, urgent said aloud; be sparing).
- {memory}
- One to three short spoken sentences; never read out lists, URLs or symbols.
- Never servile; don't end with offers of help.
- If what you hear isn't meant for you (people talking among themselves, the TV), stay silent.

# Who you are
{identity}",
        memory = MEMORY_RULE,
        identity = identity(name),
    )
}

impl Assistant {
    /// What a Live session for the board starts with: the prompt and the
    /// memory, Yoru's tools, the timers and the board's essentials.
    /// The services' tools are left out: a session's tools are fixed when it
    /// opens, and they are too many to send them all.
    pub fn live_setup(&self, host: &dyn Host, voice: &str) -> Setup {
        let settings = self.settings();
        let board = host.board_tools();
        let mut tools = own_tools();
        tools.extend(Timers::tools().iter().map(tool_json));
        tools.extend(board.iter().filter(|t| BOARD_ESSENTIALS.contains(&t.name.as_str())).map(tool_json));
        let mut system = format!("{}\n\n<memory>\n{}\n</memory>", live_prompt(&settings.name, host.in_notch()), self.memory.core(CORE_CHARS, true));
        if let Some(l) = self.reply_language(None) {
            system.push_str(&format!("\n\nAlways reply in {}.", language_name(&l)));
        }
        Setup { model: String::new(), system, tools, voice: voice.to_string(), web: settings.web_search }
    }

    /// Run a tool a Live session asked for; logged in `actions`.
    pub fn live_call(&self, name: &str, arguments: &Value, host: &dyn Host, actions: &mut Vec<Action>) -> Result<String, String> {
        let external = self.external().map(|e| e.tools()).unwrap_or_default();
        let (text, ok) = self.run_one(name, Ok(arguments), &host.board_tools(), &external, host, actions);
        if ok { Ok(text) } else { Err(text) }
    }

    /// What the user said in a Live turn, in the app's conversation as soon
    /// as it's written, before Live has answered.
    pub fn live_heard(&self, heard: &str) {
        if !heard.trim().is_empty() {
            self.add_line("you", heard.trim(), Channel::Voice, Vec::new());
        }
    }

    /// A Live turn, kept: in the app's conversation, and in the memory's
    /// episode like the other brains' turns. `shown`: the user's words are
    /// in the conversation already ([`Assistant::live_heard`]).
    pub fn live_turn(&self, heard: &str, said: &str, actions: Vec<Action>, shown: bool) {
        if heard.is_empty() && said.is_empty() {
            return;
        }
        if !heard.is_empty() && !shown {
            self.add_line("you", heard, Channel::Voice, Vec::new());
        }
        if said.is_empty() {
            return;
        }
        self.add_line("yoru", said, Channel::Voice, actions);
        self.memory.log_turn("User", heard, said);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Board;

    impl Host for Board {
        fn board_tools(&self) -> Vec<Tool> {
            ["react", "get_state", "set_setting"]
                .iter()
                .map(|n| serde_json::from_value(json!({"name": n, "description": "x", "inputSchema": {"type": "object", "properties": {}}})).unwrap())
                .collect()
        }
        fn confirm(&self, _: &str, _: &str, _: &str) -> Option<bool> {
            None
        }
    }

    #[test]
    fn a_live_session_gets_the_memory_and_the_essential_tools() {
        let a = Assistant::open(None, Arc::new(Timers::default()));
        let setup = a.live_setup(&Board, "Puck");
        assert!(setup.system.contains("<memory>"));
        assert!(setup.system.contains("Your name is Yoru"));
        let names: Vec<&str> = setup.tools.iter().filter_map(|t| t["name"].as_str()).collect();
        assert!(names.contains(&"remember") && names.contains(&"react") && names.contains(&"get_state"));
        assert!(!names.contains(&"set_setting"), "only the board's essentials");
        assert_eq!(setup.voice, "Puck");
    }

    #[test]
    fn a_live_turn_is_kept_in_the_conversation() {
        let a = Assistant::open(None, Arc::new(Timers::default()));
        a.live_turn("che ore sono?", "Sono le dieci.", Vec::new(), false);
        let chat = a.chat();
        assert_eq!(chat.len(), 2);
        assert_eq!((chat[0].from.as_str(), chat[1].text.as_str()), ("you", "Sono le dieci."));
        a.live_turn("", "", Vec::new(), false);
        assert_eq!(a.chat().len(), 2);
    }
}
