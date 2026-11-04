//! Update a board over its link (`ota/...`): through the running app's hub,
//! or straight over USB without it. A check of `yoru_core::ota` with a real
//! board.
//!
//!     cargo run -p yoru-core --example ota -- IMAGE [BOARD_ID]
//!     cargo run -p yoru-core --example ota -- status [BOARD_ID]
//!
//! IMAGE is the app (`build/yoru.bin`) or the merged image; `status` asks
//! the board where it stands (`ota/status`).

use std::time::Instant;

use yoru_core::{Board, firmware, ota};

fn main() {
    let mut args = std::env::args().skip(1);
    let path = args.next().expect("usage: ota IMAGE [BOARD_ID]");
    let board = Board::new(None, "ota-example").on(args.next());
    if path == "status" {
        let reply = board.session(|call| call("ota/status", serde_json::json!({}), std::time::Duration::from_secs(10)));
        println!("{reply:?}");
        return;
    }
    let bytes = std::fs::read(&path).expect("reading the image");
    let app = ota::app_image(&bytes).unwrap_or(&bytes);
    let version = firmware::image_info(&bytes).map(|i| i.version).unwrap_or_default();
    let started = Instant::now();
    let mut shown = -1;
    let result = board.session(|call| {
        Ok(ota::update(call, app, &version, |p| {
            if let ota::Progress::Sent(f) = p {
                let pct = (f * 100.0) as i32;
                if pct / 10 != shown / 10 {
                    shown = pct;
                    println!("{:>5.1}s  {pct}%", started.elapsed().as_secs_f32());
                }
            } else {
                println!("{:>5.1}s  restarting", started.elapsed().as_secs_f32());
            }
        }))
    });
    match result {
        Ok(Ok(())) => println!("done: {} bytes in {:.1}s", app.len(), started.elapsed().as_secs_f32()),
        Ok(Err(e)) => eprintln!("update failed: {e}"),
        Err(e) => eprintln!("no board: {e}"),
    }
}
