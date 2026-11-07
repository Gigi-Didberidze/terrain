//! terrain-indexer: the out-of-process indexer for the Terrain VS Code extension.
//! Speaks JSON-RPC 2.0 over stdio with LSP-style framing.

mod cli;
mod protocol;
mod rpc;
mod server;
mod transport;

use std::io::{self, BufReader};
use std::sync::mpsc;

/// mimalloc instead of the system allocator: parsing is allocation-heavy and the Windows heap
/// serialises threads (see DECISIONS.md D19).
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

/// Routes tree-sitter's C allocations to mimalloc too. Must run before any parser exists.
fn use_mimalloc_for_tree_sitter() {
    use std::ffi::c_void;
    unsafe extern "C" fn malloc(size: usize) -> *mut c_void {
        unsafe { libmimalloc_sys::mi_malloc(size).cast() }
    }
    unsafe extern "C" fn calloc(n: usize, size: usize) -> *mut c_void {
        unsafe { libmimalloc_sys::mi_calloc(n, size).cast() }
    }
    unsafe extern "C" fn realloc(p: *mut c_void, size: usize) -> *mut c_void {
        unsafe { libmimalloc_sys::mi_realloc(p.cast(), size).cast() }
    }
    unsafe extern "C" fn free(p: *mut c_void) {
        unsafe { libmimalloc_sys::mi_free(p.cast()) }
    }
    // SAFETY: called once at startup, before any tree-sitter object is allocated.
    unsafe {
        tree_sitter::set_allocator(Some(tree_sitter::Allocator {
            malloc,
            calloc,
            realloc,
            free,
        }))
    };
}

fn main() {
    use_mimalloc_for_tree_sitter();
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--version") {
        println!("terrain-indexer {}", env!("CARGO_PKG_VERSION"));
        return;
    }
    match args.get(1).map(String::as_str) {
        Some("index") => return cli::index(&args[2..]),
        Some("imports") => return cli::imports(&args[2..]),
        Some("analyse") => return cli::analyse(&args[2..]),
        Some("cycles") => return cli::cycles(&args[2..]),
        Some("history") => return cli::history(&args[2..]),
        _ => {}
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
