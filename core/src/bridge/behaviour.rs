//! What a session does differently for each kind of board. The session
//! handles the link, the handshake, the voice and sending snapshots, the
//! same for every board; a [`BoardBehaviour`], picked from the board's
//! `hello`, does the rest: its settings, what of the shared snapshot it
//! gets, and whether its eyes follow the pointer.

use std::io;
use std::sync::Arc;

use crate::link::{Hello, Link};
use crate::profile::BoardKind;
use crate::snapshot::Snapshot;

use super::{BridgeConfig, EventSink, dualeye::DualEye, yoru::Yoru};

/// What a behaviour works with: the board's link, the bridge's config, and
/// where events go.
pub(super) struct Ctx<'a> {
    pub link: &'a Link,
    pub config: &'a BridgeConfig,
    pub on_event: &'a EventSink,
}

/// Whether the pointer should go to the board's eyes right now; asked by
/// the session's gaze thread, a few times a second.
pub(super) type GazeWanted = Arc<dyn Fn() -> bool + Send + Sync>;

pub(super) trait BoardBehaviour: Send {
    /// The board answered `hello` (after connecting, or a reboot). Only an
    /// I/O error ends the session; anything else is logged.
    fn on_ready(&mut self, ctx: &Ctx, hello: &Hello) -> io::Result<()>;

    /// The board restarted: it forgot what it was sent.
    fn on_reboot(&mut self);

    /// Each tick while the board is ready, before the snapshot: settings to
    /// read back (`changed`: a hub client or the voice changed some) and to push.
    fn sync(&mut self, ctx: &Ctx, changed: bool) -> io::Result<()>;

    /// Make the shared snapshot this board's, sending what it needs first
    /// (a music cover). `ready`: the board answered its `hello`.
    fn tailor(&mut self, ctx: &Ctx, snapshot: &mut Snapshot, ready: bool) -> io::Result<()>;

    /// When the pointer goes to its eyes.
    fn gaze(&self, config: &BridgeConfig) -> GazeWanted;
}

/// The behaviour for a `kind` board.
pub(super) fn for_kind(kind: BoardKind) -> Box<dyn BoardBehaviour> {
    match kind {
        BoardKind::DualEye => Box::new(DualEye::default()),
        BoardKind::Yoru => Box::new(Yoru),
    }
}
