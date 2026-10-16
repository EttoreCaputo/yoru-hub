//! The kinds of board the host drives, and what differs between them: how
//! each one names itself on the link and on WiFi, its screens, and how the
//! voice agent and MCP clients describe it. Shared code asks the profile
//! instead of spelling out one board's names.
//!
//! A board's kind is known from its `hello` (`board`); both kinds use the
//! same USB device, so not before.

use serde::{Deserialize, Serialize};

use crate::link::Hello;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BoardKind {
    /// ESP32-S3 DualEye: two round 240×240 LCDs with watch faces.
    #[default]
    DualEye,
    /// Waveshare ESP32-S3-Touch-AMOLED-1.75C: one round AMOLED, a pet's eyes.
    Yoru,
}

/// What one kind of board is, for the code that talks to it.
#[derive(Debug, PartialEq, Eq)]
pub struct BoardProfile {
    pub kind: BoardKind,
    /// The name the user knows it by.
    pub name: &'static str,
    /// `board` in its `hello`.
    pub hello_board: &'static str,
    /// The mDNS service it announces on WiFi.
    pub mdns_service: &'static str,
    /// Labels of the WiFi handshake's proofs, host's then board's.
    pub proof_labels: (&'static str, &'static str),
    /// Labels of the WiFi session keys, host-to-board then board-to-host.
    pub key_labels: (&'static str, &'static str),
    /// It shows watch faces (`set_face`, `set_rotation`) on two screens.
    pub faces: bool,
    /// The key of the version line firmware before protocol v2 prints
    /// (`{"dualeye":"0.3.0",…}`), if it ever had one.
    pub legacy_version_key: Option<&'static str>,
    /// Its folder under `firmware/` and its file names on this computer.
    pub slug: &'static str,
    /// The ESP-IDF project its firmware is built as: the `project` in a
    /// flash image's app descriptor, which tells whose image it is.
    pub image_project: &'static str,
    /// The board tool that plays a short scene of the eyes by name (`name`:
    /// one of DualEye's `play_eyes` scenes), before the pet speaks up.
    pub scene_tool: &'static str,
    /// It shows Claude Code's permission requests with Allow and Deny
    /// (`show_approval`, `hide_approval`) and answers with an `approval`
    /// notification.
    pub approvals: bool,
    /// Board tools only the hub calls, kept from MCP clients: a model has no
    /// business putting a permission card on the screen.
    pub hub_only_tools: &'static [&'static str],
    /// The body the voice agent lives in, for its prompts.
    pub agent_body: &'static str,
    /// What MCP clients are told about the board.
    pub mcp_about: &'static str,
}

const DUALEYE: BoardProfile = BoardProfile {
    kind: BoardKind::DualEye,
    name: "DualEye",
    hello_board: "dualeye",
    mdns_service: "_dualeye._tcp.local.",
    proof_labels: ("dualeye host", "dualeye board"),
    key_labels: ("dualeye h2b", "dualeye b2h"),
    faces: true,
    legacy_version_key: Some("dualeye"),
    slug: "dualeye",
    image_project: "esp32s3-dualeye-pcmonitor",
    scene_tool: "play_eyes",
    approvals: false,
    hub_only_tools: &[],
    agent_body: "a board with two round displays, the left one and the right one, a microphone and a speaker",
    mcp_about: "DualEye is a small USB display on the user's desk with two round screens: \
`left` and `right`; any face goes on either one: the computer's CPU or GPU (set_face's source), network, disk, battery, \
Claude Code usage (claude and clawd), a picture the user uploaded in the app (image), what's playing (music) \
or a pair of eyes that follow the mouse (eyes). \
The board's tools change what the screens show; show_text puts a short ASCII message on them.",
};

const YORU: BoardProfile = BoardProfile {
    kind: BoardKind::Yoru,
    name: "Yoru",
    hello_board: "yoru-amoled-1.75c",
    mdns_service: "_yoru._tcp.local.",
    proof_labels: ("yoru host", "yoru board"),
    key_labels: ("yoru h2b", "yoru b2h"),
    faces: false,
    legacy_version_key: None,
    slug: "yoru",
    image_project: "yoru",
    scene_tool: "react",
    approvals: true,
    hub_only_tools: &["show_approval", "hide_approval"],
    agent_body: "a small round screen where you appear as a matte sphere with two eyes and two floating hands, \
with a microphone and a speaker",
    mcp_about: "Yoru is a desk pet on the user's desk: a round AMOLED screen where it lives as a small sphere \
with two eyes and floating hands, with a touch screen, a motion sensor, two microphones and a speaker. \
It has moods of its own, driven by the user's touches, its pet state and this computer. \
The board's tools set it up (get_settings and set_setting, brightness, volume, the mic, the wake word), \
make it react for a few seconds (react) or play a sound, read its state (get_state), and open its settings menu on the screen (show_menu).",
};

impl BoardKind {
    pub const ALL: [BoardKind; 2] = [BoardKind::DualEye, BoardKind::Yoru];

    pub fn profile(self) -> &'static BoardProfile {
        match self {
            BoardKind::DualEye => &DUALEYE,
            BoardKind::Yoru => &YORU,
        }
    }

    /// The kind `board` in a `hello` names. Firmware that didn't say is DualEye,
    /// the only board there was; one this host doesn't know is `None`.
    pub fn from_board(board: Option<&str>) -> Option<BoardKind> {
        match board {
            None => Some(BoardKind::DualEye),
            Some(b) => Self::ALL.into_iter().find(|k| k.profile().hello_board == b),
        }
    }

    /// The kind of the board that sent `hello`; an unknown one is treated as
    /// DualEye, whose protocol it speaks.
    pub fn from_hello(hello: &Hello) -> BoardKind {
        Self::from_board(hello.board.as_deref()).unwrap_or_default()
    }

    /// Whose firmware a flash image is, from its app descriptor's `project`.
    pub fn from_image_project(project: &str) -> Option<BoardKind> {
        Self::ALL.into_iter().find(|k| k.profile().image_project == project)
    }

    /// Some board's tool `name` is the hub's own, whichever board lists it.
    pub fn hub_only(name: &str) -> bool {
        Self::ALL.into_iter().any(|k| k.profile().hub_only_tools.contains(&name))
    }

    /// From a kind's `slug` (`dualeye`, `yoru`), as the CLI and the hub take it.
    pub fn from_slug(slug: &str) -> Option<BoardKind> {
        Self::ALL.into_iter().find(|k| k.profile().slug.eq_ignore_ascii_case(slug))
    }
}

impl std::fmt::Display for BoardKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.profile().name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hello(board: Option<&str>) -> Hello {
        Hello {
            protocol: 2,
            firmware: "1.0.0".into(),
            idf: None,
            board: board.map(str::to_string),
            max_payload: None,
            channels: vec![],
            capabilities: vec![],
            id: None,
            transport: None,
        }
    }

    #[test]
    fn kind_from_hello() {
        assert_eq!(BoardKind::from_hello(&hello(Some("dualeye"))), BoardKind::DualEye);
        assert_eq!(BoardKind::from_hello(&hello(Some("yoru-amoled-1.75c"))), BoardKind::Yoru);
        assert_eq!(BoardKind::from_hello(&hello(None)), BoardKind::DualEye);
        assert_eq!(BoardKind::from_board(Some("toaster")), None);
    }

    #[test]
    fn names_do_not_clash() {
        let [a, b] = BoardKind::ALL.map(BoardKind::profile);
        assert_ne!(a.mdns_service, b.mdns_service);
        assert_ne!(a.proof_labels, b.proof_labels);
        assert_ne!(a.key_labels, b.key_labels);
        assert_ne!(a.slug, b.slug);
    }

    #[test]
    fn serde_and_slugs() {
        assert_eq!(serde_json::to_value(BoardKind::Yoru).unwrap(), "yoru");
        assert_eq!(serde_json::from_value::<BoardKind>("dualeye".into()).unwrap(), BoardKind::DualEye);
        assert_eq!(BoardKind::from_slug("Yoru"), Some(BoardKind::Yoru));
        assert_eq!(BoardKind::from_slug("nope"), None);
        assert_eq!(BoardKind::from_image_project("yoru"), Some(BoardKind::Yoru));
        assert_eq!(BoardKind::from_image_project("esp32s3-dualeye-pcmonitor"), Some(BoardKind::DualEye));
    }
}
