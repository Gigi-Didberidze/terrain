//! Request dispatch. The reader thread only decodes; every request runs on the rayon pool so a
//! slow request never blocks the next one. Indexing runs on its own thread.

use crate::protocol::{self as p, method, notification};
use crate::rpc::{parse_params, Incoming, Outbox, RpcError, RpcResult};
use base64::Engine as _;
use compass_core::layout::{WORLD_H, WORLD_W};
use compass_core::model::{flags, NodeKind};
use compass_core::store::Store;
use compass_core::tiles;
use compass_engine::{self as engine, Engine, Events, Options, Phase, Shared};
use parking_lot::RwLock;
use serde_json::Value;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

#[derive(PartialEq, Eq)]
pub enum Flow {
    Continue,
    Exit,
}

pub struct Server {
    outbox: Outbox,
    started: Instant,
    state: Arc<RwLock<Option<Shared>>>,
}

struct RpcEvents(Outbox);

impl Events for RpcEvents {
    fn progress(&self, phase: Phase, done: u64, total: u64) {
        self.0.notify(
            notification::INDEX_PROGRESS,
            &p::ProgressParams {
                phase: phase_dto(phase),
                done,
                total,
                message: None,
            },
        );
    }
    fn changed(&self, epoch: u64, layout: bool) {
        self.0.notify(
            notification::MAP_CHANGED,
            &p::MapChangedParams { epoch, layout },
        );
    }
}

impl Server {
    pub fn new(outbox: Outbox) -> Self {
        Self {
            outbox,
            started: Instant::now(),
            state: Arc::new(RwLock::new(None)),
        }
    }

    pub fn handle_raw(&mut self, body: &[u8]) -> Flow {
        let msg: Incoming = match serde_json::from_slice(body) {
            Ok(m) => m,
            Err(e) => {
                self.outbox.respond(
                    Value::Null,
                    Err(RpcError::new(RpcError::PARSE, e.to_string())),
                );
                return Flow::Continue;
            }
        };
        let Some(m) = msg.method.clone() else {
            return Flow::Continue;
        };
        if m == "exit" {
            return Flow::Exit;
        }
        let Some(id) = msg.id else {
            return Flow::Continue;
        };
        match m.as_str() {
            method::INITIALIZE => {
                let r = self.initialize(msg.params);
                self.outbox.respond(id, r);
            }
            method::PING => {
                let r = to_value(p::PingResult {
                    uptime_ms: self.started.elapsed().as_millis() as u64,
                });
                self.outbox.respond(id, r);
            }
            method::SHUTDOWN => self.outbox.respond(id, Ok(Value::Null)),
            _ => {
                let outbox = self.outbox.clone();
                let state = self.state.read().clone();
                rayon::spawn(move || {
                    let r = match state {
                        Some(s) => dispatch(&s, &m, msg.params, &outbox),
                        None => Err(RpcError::new(RpcError::NOT_INITIALIZED, "not initialized")),
                    };
                    outbox.respond(id, r);
                });
            }
        }
        Flow::Continue
    }

    fn initialize(&mut self, params: Value) -> RpcResult {
        let ip: p::InitializeParams = parse_params(params)?;
        let root = PathBuf::from(&ip.workspace_root);
        let store = if ip.store_path.is_empty() {
            None
        } else {
            match Store::open(std::path::Path::new(&ip.store_path)) {
                Ok(s) => Some(s),
                Err(e) => {
                    eprintln!(
                        "could not open store {}: {e}; continuing without persistence",
                        ip.store_path
                    );
                    None
                }
            }
        };
        let shared = Engine::new(&root, store, Options::default()).shared();
        *self.state.write() = Some(shared.clone());
        let events = RpcEvents(self.outbox.clone());
        std::thread::Builder::new()
            .name("indexer".into())
            .spawn(move || {
                let t = engine::run_index(&shared, &events);
                eprintln!(
                    "indexed {} files: warm {:.0} ms, first map {:.0} ms, total {:.0} ms",
                    shared.read().graph.file_count(),
                    t.warm_load_ms,
                    t.first_map_ms.unwrap_or(0.0),
                    t.total_ms
                );
            })
            .map_err(RpcError::internal)?;
        to_value(p::InitializeResult {
            server_version: env!("CARGO_PKG_VERSION").into(),
            protocol_version: p::PROTOCOL_VERSION,
        })
    }
}

fn dispatch(s: &Shared, m: &str, params: Value, outbox: &Outbox) -> RpcResult {
    match m {
        method::MAP_INFO => {
            let e = s.read();
            to_value(p::MapInfo {
                epoch: e.graph.epoch,
                files: e.graph.file_count() as u64,
                phase: phase_dto(e.phase),
                world_w: WORLD_W,
                world_h: WORLD_H,
                tile_px: tiles::TILE_PX,
                min_px: tiles::MIN_PX,
                max_zoom: tiles::MAX_ZOOM,
                root_name: e.graph.node(compass_core::Graph::ROOT).name.to_string(),
            })
        }
        method::MAP_TILES => {
            let tp: p::TilesParams = parse_params(params)?;
            let e = s.read();
            let b64 = base64::engine::general_purpose::STANDARD;
            let tiles = tp
                .tiles
                .iter()
                .take(256)
                .map(|k| p::Tile {
                    z: k.z,
                    x: k.x,
                    y: k.y,
                    data: b64.encode(e.tile(k.z, k.x, k.y)),
                })
                .collect();
            to_value(p::TilesResult {
                epoch: e.graph.epoch,
                tiles,
            })
        }
        method::NODE_INFO => {
            let sp: p::SlotParams = parse_params(params)?;
            let e = s.read();
            let info =
                node_info(&e, sp.slot).ok_or_else(|| RpcError::invalid_params("no such node"))?;
            to_value(info)
        }
        method::NODE_LOCATE => {
            let lp: p::LocateParams = parse_params(params)?;
            let e = s.read();
            let node = e
                .locate(&lp.path, lp.line)
                .and_then(|slot| node_info(&e, slot));
            to_value(p::LocateResult { node })
        }
        method::SEARCH => {
            let sp: p::SearchParams = parse_params(params)?;
            let t = Instant::now();
            let e = s.read();
            let hits = e
                .search
                .search(&sp.query, sp.limit.min(500) as usize, |_| 0.0)
                .into_iter()
                .filter_map(|h| {
                    let n = e.graph.get(h.slot)?;
                    let file = if n.kind.is_symbol() { n.file } else { h.slot };
                    Some(p::SearchHit {
                        slot: h.slot,
                        kind: kind_dto(n.kind),
                        name: n.name.to_string(),
                        path: e.graph.path(file).unwrap_or("").to_string(),
                        line: n.start_line,
                        score: h.score as f64,
                    })
                })
                .collect();
            to_value(p::SearchResult {
                hits,
                elapsed_ms: t.elapsed().as_secs_f64() * 1000.0,
            })
        }
        method::FILES_CHANGED => {
            let fp: p::FilesChangedParams = parse_params(params)?;
            let t = Instant::now();
            let changed = engine::apply_changes(s, &fp.paths, &RpcEvents(outbox.clone()));
            to_value(p::FilesChangedResult {
                changed,
                elapsed_ms: t.elapsed().as_secs_f64() * 1000.0,
            })
        }
        method::INDEX_STATUS => {
            let e = s.read();
            let t = &e.timings;
            to_value(p::StatusResult {
                phase: phase_dto(e.phase),
                files: e.graph.file_count() as u64,
                epoch: e.graph.epoch,
                timings: p::Timings {
                    warm_load_ms: t.warm_load_ms,
                    first_map_ms: t.first_map_ms.unwrap_or(0.0),
                    discover_ms: t.discover_ms,
                    hash_ms: t.hash_ms,
                    total_ms: t.total_ms,
                },
                rss_mb: memory_stats::memory_stats()
                    .map(|m| m.physical_mem as f64 / 1048576.0)
                    .unwrap_or(0.0),
            })
        }
        _ => Err(RpcError::new(
            RpcError::METHOD_NOT_FOUND,
            format!("unknown method {m}"),
        )),
    }
}

fn node_info(e: &Engine, slot: u32) -> Option<p::NodeInfo> {
    let i = e.node_info(slot)?;
    let mut ancestors = Vec::new();
    let mut cur = i.parent;
    while let Some(a) = cur {
        ancestors.push(a);
        let n = e.graph.node(a);
        cur = (n.parent != compass_core::model::NONE).then_some(n.parent);
    }
    Some(p::NodeInfo {
        slot: i.slot,
        id: i.id.hex(),
        kind: kind_dto(i.kind),
        name: i.name,
        qualified: i.qualified,
        path: i.path,
        lang: i.lang.as_str().into(),
        loc: i.loc,
        start_line: i.start_line,
        end_line: i.end_line,
        files: i.files,
        generated: i.flags & flags::GENERATED != 0,
        test: i.flags & flags::TEST != 0,
        rect: i.rect.map(|r| p::Rect {
            x: r.x,
            y: r.y,
            w: r.w,
            h: r.h,
        }),
        ancestors,
    })
}

pub fn phase_dto(ph: Phase) -> p::IndexPhase {
    match ph {
        Phase::Discover => p::IndexPhase::Discover,
        Phase::Hash => p::IndexPhase::Hash,
        Phase::Parse => p::IndexPhase::Parse,
        Phase::Resolve => p::IndexPhase::Resolve,
        Phase::Analyse => p::IndexPhase::Analyse,
        Phase::Persist => p::IndexPhase::Persist,
        Phase::History => p::IndexPhase::History,
        Phase::Idle => p::IndexPhase::Idle,
    }
}

pub fn kind_dto(k: NodeKind) -> p::NodeKind {
    match k {
        NodeKind::Dir => p::NodeKind::Dir,
        NodeKind::File => p::NodeKind::File,
        NodeKind::Module => p::NodeKind::Module,
        NodeKind::Class => p::NodeKind::Class,
        NodeKind::Function => p::NodeKind::Function,
        NodeKind::Method => p::NodeKind::Method,
        NodeKind::Type => p::NodeKind::Type,
        NodeKind::Constant => p::NodeKind::Constant,
    }
}

pub fn to_value<T: serde::Serialize>(v: T) -> RpcResult {
    serde_json::to_value(v).map_err(RpcError::internal)
}
