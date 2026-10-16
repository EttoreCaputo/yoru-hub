//! A Yoru's side of the session: one round screen, no faces. It reads the
//! snapshot for the pet's mood only (see Yoru's `docs/protocol.md`), keeps
//! its settings itself (`get_settings`, `set_setting`), and its eyes follow
//! the pointer whenever the host sends it: the board decides when to look
//! (not while a finger is on the screen, in a conversation or asleep).

use std::io;
use std::sync::Arc;

use crate::link::Hello;
use crate::snapshot::Snapshot;

use super::BridgeConfig;
use super::behaviour::{BoardBehaviour, Ctx, GazeWanted};

pub(super) struct Yoru;

impl BoardBehaviour for Yoru {
    fn on_ready(&mut self, _ctx: &Ctx, _hello: &Hello) -> io::Result<()> {
        Ok(())
    }

    fn on_reboot(&mut self) {}

    fn sync(&mut self, _ctx: &Ctx, _changed: bool) -> io::Result<()> {
        Ok(())
    }

    fn tailor(&mut self, _ctx: &Ctx, snapshot: &mut Snapshot, _ready: bool) -> io::Result<()> {
        // No faces to name, no cover to show: the music's state is enough for the pet.
        snapshot.face = None;
        snapshot.rot = None;
        if let Some(music) = snapshot.music.as_mut() {
            music.art = None;
        }
        Ok(())
    }

    fn gaze(&self, _config: &BridgeConfig) -> GazeWanted {
        Arc::new(|| true)
    }
}
