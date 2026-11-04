//! Send one request to a board, through the running app's hub or straight
//! over USB, and print the reply: for the board's test hooks (`debug/...`,
//! Yoru's `docs/protocol.md`).
//!
//!     cargo run -p yoru-core --example rpc -- METHOD [PARAMS_JSON] [BOARD_ID]
//!     cargo run -p yoru-core --example rpc -- debug/friends '{"on":true}'

use std::time::Duration;

use yoru_core::Board;

fn main() {
    let mut args = std::env::args().skip(1);
    let method = args.next().expect("usage: rpc METHOD [PARAMS_JSON] [BOARD_ID]");
    let params: serde_json::Value = args.next().map_or(serde_json::json!({}), |p| serde_json::from_str(&p).expect("PARAMS_JSON"));
    let board = Board::new(None, "rpc-example").on(args.next());
    match board.session(|call| call(&method, params, Duration::from_secs(20))) {
        Ok(reply) => println!("{}", serde_json::to_string_pretty(&reply).unwrap_or_default()),
        Err(e) => eprintln!("{e}"),
    }
}
