//! A DualEye's side of the session: two screens, each with a watch face and
//! a rotation, and the music face's cover.
//!
//! Faces and rotation live on the board (NVS), so by default the bridge
//! reads them with `get_state` at the handshake and adopts them, raising
//! [`BridgeEvent::Settings`]: a change made while no bridge ran (say, over
//! MCP) survives. From then on, whatever the frontend picks is pushed with
//! board tools. When a hub client or the voice changes a face or a rotation,
//! the board's state is read back, so the frontend follows. Each DualEye
//! keeps its own under its id ([`BridgeConfig::screens`]).
//!
//! While a screen shows the music face, the cover goes to the board
//! (`music/art`) before the snapshot that names it. While one shows the eyes
//! face, the pointer goes to them.

use std::io;
use std::sync::{Arc, Mutex};

use serde_json::json;

use crate::link::{CallError, Hello, Link};
use crate::music;
use crate::snapshot::{Face, Faces, Rotation, Rotations, Snapshot, Source, Sources};

use super::behaviour::{BoardBehaviour, Ctx, GazeWanted};
use super::{BridgeConfig, BridgeEvent, EventSink, ScreenSettings, TOOL_TIMEOUT};

#[derive(Default)]
pub(super) struct DualEye {
    /// The board's id, from its `hello`: whose faces and rotation it shows.
    id: Arc<Mutex<Option<String>>>,
    /// What the board was last given; `None` makes the next tick push ours.
    pushed: Option<(Faces, Rotations)>,
    /// Its firmware shows covers (`music` capability).
    has_music: bool,
    /// The cover the board has (it forgets them when it reboots), and one it refused.
    cover_on_board: Option<u32>,
    cover_refused: Option<u32>,
}

impl BoardBehaviour for DualEye {
    fn on_ready(&mut self, ctx: &Ctx, hello: &Hello) -> io::Result<()> {
        *self.id.lock().unwrap() = hello.id.clone();
        self.has_music = hello.capabilities.iter().any(|c| c == "music");
        self.cover_on_board = None;
        if ctx.config.adopt_board_settings {
            // Otherwise `pushed` stays empty and ours are pushed on the next tick.
            adopt_settings(ctx.link, ctx.config, self.id().as_deref(), &mut self.pushed, ctx.on_event)?;
        }
        Ok(())
    }

    fn on_reboot(&mut self) {
        self.pushed = None;
        self.cover_on_board = None;
    }

    fn sync(&mut self, ctx: &Ctx, changed: bool) -> io::Result<()> {
        if changed {
            adopt_settings(ctx.link, ctx.config, self.id().as_deref(), &mut self.pushed, ctx.on_event)?;
        }
        let screens = ctx.config.screens(self.id().as_deref());
        let want = (screens.faces, screens.rotation);
        if self.pushed != Some(want) {
            match push_settings(ctx.link, want.0, want.1) {
                Ok(()) => self.pushed = Some(want),
                Err(CallError::Io(e)) => return Err(e),
                // Retried on the next tick.
                Err(e) => (ctx.on_event)(BridgeEvent::BoardLog { line: format!("host: settings not applied: {e}") }),
            }
        }
        Ok(())
    }

    fn tailor(&mut self, ctx: &Ctx, snapshot: &mut Snapshot, ready: bool) -> io::Result<()> {
        let config = ctx.config;
        let screens = config.screens(self.id().as_deref());
        snapshot.face = Some(screens.faces);
        snapshot.rot = Some(screens.rotation).filter(|r| !r.is_upright());
        // Watched for the music face, from the next snapshot on.
        if snapshot.face.is_some_and(|f| f.shows(Face::Music)) {
            config.music.want();
        }
        let Some(view) = snapshot.music.as_mut().filter(|_| ready && self.has_music) else { return Ok(()) };
        // The cover first, so the snapshot naming it finds it there.
        match config.music.cover().filter(|c| Some(c.id) == view.art) {
            Some(cover) if self.cover_on_board != Some(cover.id) && self.cover_refused != Some(cover.id) => {
                match music::upload_cover(|m, p, t| ctx.link.call(m, p, t), &cover) {
                    Ok(()) => self.cover_on_board = Some(cover.id),
                    Err(CallError::Io(e)) => return Err(e),
                    Err(e) => {
                        self.cover_refused = Some(cover.id);
                        view.art = None;
                        (ctx.on_event)(BridgeEvent::BoardLog { line: format!("host: cover not sent: {e}") });
                    }
                }
            }
            Some(cover) if self.cover_on_board != Some(cover.id) => view.art = None,
            _ => {}
        }
        Ok(())
    }

    fn gaze(&self, config: &BridgeConfig) -> GazeWanted {
        let (config, id) = (config.clone(), self.id.clone());
        Arc::new(move || config.screens(id.lock().unwrap().as_deref()).faces.shows(Face::Eyes))
    }
}

impl DualEye {
    fn id(&self) -> Option<String> {
        self.id.lock().unwrap().clone()
    }
}

fn push_settings(link: &Link, faces: Faces, rotation: Rotations) -> Result<(), CallError> {
    let screens = [("left", faces.cpu, faces.src.cpu, rotation.cpu), ("right", faces.gpu, faces.src.gpu, rotation.gpu)];
    for (screen, face, source, rot) in screens {
        let calls = [
            // A firmware before 1.1 ignores `source`.
            ("set_face", json!({"screen": screen, "face": face.name(), "source": source.name()})),
            ("set_rotation", json!({"screen": screen, "degrees": rot.degrees()})),
        ];
        for (tool, args) in calls {
            let result = link.call_tool(tool, args, TOOL_TIMEOUT)?;
            if result.is_error {
                return Err(CallError::Invalid(format!("{tool}: {}", result.text())));
            }
        }
    }
    Ok(())
}

/// Take the board's faces and rotation as ours. Only an I/O error is returned;
/// on any other failure `pushed` is left alone.
fn adopt_settings(link: &Link, config: &BridgeConfig, id: Option<&str>, pushed: &mut Option<(Faces, Rotations)>, on_event: &EventSink) -> io::Result<()> {
    match read_settings(link) {
        Ok((faces, rotation)) => {
            config.set_screens(id, ScreenSettings { faces, rotation });
            *pushed = Some((faces, rotation));
            on_event(BridgeEvent::Settings { faces, rotation });
        }
        Err(CallError::Io(e)) => return Err(e),
        Err(e) => on_event(BridgeEvent::BoardLog { line: format!("host: could not read the board's settings: {e}") }),
    }
    Ok(())
}

/// Faces and rotation as the board has them, from `get_state`.
fn read_settings(link: &Link) -> Result<(Faces, Rotations), CallError> {
    let state = link.call_tool("get_state", json!({}), TOOL_TIMEOUT)?;
    let screens = state.structured_content.as_ref().and_then(|s| s.get("screens")).ok_or_else(|| CallError::Invalid("get_state: no screens".into()))?;
    // Before firmware 1.1 the left screen always showed the CPU, the right one the GPU.
    let screen = |name: &str, usual: Source| -> Result<(Face, Source, Rotation), CallError> {
        let s = &screens[name];
        let face = s["face"].as_str().and_then(|f| f.parse().ok());
        let source = s["source"].as_str().and_then(|f| f.parse().ok()).unwrap_or(usual);
        let rotation = s["rotation"].as_u64().and_then(|d| Rotation::from_degrees(d as u16));
        let (face, rotation) = face.zip(rotation).ok_or_else(|| CallError::Invalid(format!("get_state: {name} screen")))?;
        Ok((face, source, rotation))
    };
    let (left, right) = (screen("left", Source::Cpu)?, screen("right", Source::Gpu)?);
    let faces = Faces { cpu: left.0, gpu: right.0, src: Sources { cpu: left.1, gpu: right.1 } };
    Ok((faces, Rotations { cpu: left.2, gpu: right.2 }))
}
