//! compass-indexer: the out-of-process indexer for the Compass VS Code extension.
//! Speaks JSON-RPC 2.0 over stdio with LSP-style framing.

mod protocol;
mod rpc;
mod server;
mod transport;

use std::io::{self, BufReader};
use std::sync::mpsc;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--version") {
        println!("compass-indexer {}", env!("CARGO_PKG_VERSION"));
        return;
    }

    let (tx, rx) = mpsc::channel::<Vec<u8>>();
    let writer = std::thread::Builder::new()
        .name("rpc-writer".into())
        .spawn(move || {
            let stdout = io::stdout();
            let mut out = io::BufWriter::new(stdout.lock());
            for msg in rx {
                if transport::write_message(&mut out, &msg).is_err() {
                    break;
                }
            }
        })
        .expect("spawn writer");

    let outbox = rpc::Outbox(tx);
    let mut server = server::Server::new(outbox);
    let stdin = io::stdin();
    let mut reader = BufReader::new(stdin.lock());
    while let Ok(Some(body)) = transport::read_message(&mut reader) {
        if server.handle_raw(&body) == server::Flow::Exit {
            break;
        }
    }
    drop(server);
    let _ = writer.join();
}
