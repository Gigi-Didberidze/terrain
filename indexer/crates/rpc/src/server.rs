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

#[derive(Clone)]
struct RpcEvents(Outbox);

impl Events for RpcEvents {
    fn boxed(&self) -> Box<dyn Events> {
        Box::new(self.clone())
    }
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
        let options = Options {
            cap_data_files: ip.cap_data_files.unwrap_or(true),
            ..Options::default()
        };
        let shared = Engine::new(&root, store, options).shared();
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
            let t = Instant::now();
            let e = s.read();
            let locked = t.elapsed();
            e.with_layout(|_| ());
            let laid = t.elapsed();
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
                .collect::<Vec<_>>();
            if std::env::var_os("COMPASS_TRACE").is_some() {
                eprintln!(
                    "tiles: {} tiles, lock {:.0} ms, layout {:.0} ms, total {:.0} ms",
                    tiles.len(),
                    locked.as_secs_f64() * 1000.0,
                    laid.as_secs_f64() * 1000.0,
                    t.elapsed().as_secs_f64() * 1000.0
                );
            }
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
        method::NODE_EDGES => {
            let ep: p::EdgesParams = parse_params(params)?;
            let e = s.read();
            if e.graph.get(ep.slot).is_none() {
                return Err(RpcError::invalid_params("no such node"));
            }
            to_value(edges(&e, ep.slot, ep.limit.min(1000) as usize))
        }
        method::SETTINGS_UPDATE => {
            let sp: p::SettingsParams = parse_params(params)?;
            let mut e = s.write();
            if e.options.cap_data_files != sp.cap_data_files {
                e.options.cap_data_files = sp.cap_data_files;
                // The layout reads the option; bumping the epochs makes every view refetch.
                e.graph.layout_epoch += 1;
                e.graph.epoch += 1;
                let epoch = e.graph.epoch;
                drop(e);
                RpcEvents(outbox.clone()).changed(epoch, true);
            }
            Ok(Value::Null)
        }
        method::NODE_CONTEXT => {
            let cp: p::ContextParams = parse_params(params)?;
            let e = s.read();
            let node =
                node_info(&e, cp.slot).ok_or_else(|| RpcError::invalid_params("no such node"))?;
            let c = e
                .context(cp.slot, cp.limit.clamp(1, 500) as usize)
                .unwrap_or_default();
            to_value(p::ContextResult {
                node,
                callers: refs(c.callers),
                callees: refs(c.callees),
                importers: refs(c.importers),
                imports: refs(c.imports),
                tests: refs(c.tests),
                blast: refs(c.blast),
                blast_count: c.blast_count,
            })
        }
        method::FILE_LENSES => {
            let pp: p::PathParams = parse_params(params)?;
            let e = s.read();
            let lenses = e
                .lenses(&pp.path)
                .into_iter()
                .map(|l| p::LensItem {
                    slot: l.slot,
                    line: l.line,
                    callers: l.callers,
                    blast: l.blast,
                    tests: l.tests,
                    complexity: l.complexity,
                })
                .collect();
            to_value(p::LensesResult { lenses })
        }
        method::HISTORY_SUMMARY => {
            let e = s.read();
            let h = e.history.clone();
            to_value(p::HistorySummary {
                available: h.head.is_some(),
                commits: h.commits as u32,
                head: h.head.clone(),
                top_authors: h.top_authors.clone(),
                max_churn: h.max_churn,
                churn_days: compass_engine::history::CHURN_DAYS as u32,
                coupled_files: h.coupling.len() as u32,
                hidden_pairs: e.hidden_pairs(usize::MAX).len() as u32,
            })
        }
        method::HISTORY_FILE => {
            let pp: p::PathParams = parse_params(params)?;
            let e = s.read();
            let h = e.history.clone();
            let owner = h.owner.get(&pp.path);
            let partners = e
                .partners(&pp.path)
                .into_iter()
                .map(|(partner, slot, linked)| p::CouplingView {
                    node: node_ref(e.node_ref(slot)),
                    degree: partner.degree as f64,
                    shared: partner.shared,
                    hidden: !linked,
                    rect: e.node_info(slot).and_then(|i| i.rect).map(rect_dto),
                })
                .collect();
            to_value(p::HistoryFileResult {
                churn: h.churn.get(&pp.path).copied().unwrap_or(0),
                commits: h.total.get(&pp.path).copied().unwrap_or(0),
                owner: owner.map(|o| o.0.clone()),
                owner_share: owner.map(|o| o.1 as f64).unwrap_or(0.0),
                partners,
            })
        }
        method::HISTORY_BLAME => {
            let pp: p::PathParams = parse_params(params)?;
            let authors = s
                .read()
                .blame(&pp.path)
                .into_iter()
                .map(|(author, lines)| p::AuthorLines { author, lines })
                .collect();
            to_value(p::BlameResult { authors })
        }
        method::HISTORY_HIDDEN => {
            let lp: p::LimitParams = parse_params(params)?;
            let e = s.read();
            let all = e.hidden_pairs(usize::MAX);
            let pairs = all
                .iter()
                .take(lp.limit.min(5000) as usize)
                .map(|(a, b, partner)| p::HiddenPair {
                    a: node_ref(e.node_ref(*a)),
                    b: node_ref(e.node_ref(*b)),
                    degree: partner.degree as f64,
                    shared: partner.shared,
                })
                .collect();
            to_value(p::HiddenResult {
                pairs,
                total: all.len() as u32,
            })
        }
        method::HISTORY_REFRESH => {
            let changed = compass_engine::run_history(s, &RpcEvents(outbox.clone()));
            to_value(p::RefreshResult { changed })
        }
        method::ANALYSIS_MISPLACED => {
            let lp: p::LimitParams = parse_params(params)?;
            let e = s.read();
            let a = e.analysis.clone();
            let items = a
                .misplaced
                .iter()
                .take(lp.limit.min(5000) as usize)
                .map(|m| p::MisplacedView {
                    node: node_ref(e.node_ref(m.file)),
                    home: node_ref(e.node_ref(m.home)),
                    share: m.share as f64,
                })
                .collect();
            to_value(p::MisplacedResult {
                items,
                total: a.misplaced.len() as u32,
            })
        }
        method::ANALYSIS_CYCLES => {
            let e = s.read();
            let a = e.analysis.clone();
            to_value(p::CyclesResult {
                files: a.file_cycles.iter().map(|c| cycle_view(&e, c)).collect(),
                dirs: a.dir_cycles.iter().map(|c| cycle_view(&e, c)).collect(),
            })
        }
        method::ANALYSIS_DEAD => {
            let lp: p::LimitParams = parse_params(params)?;
            let e = s.read();
            let a = e.analysis.clone();
            let items = a
                .dead
                .iter()
                .take(lp.limit.min(5000) as usize)
                .map(|&s| node_ref(e.node_ref(s)))
                .collect();
            to_value(p::DeadResult {
                items,
                total: a.dead.len() as u32,
            })
        }
        method::ANALYSIS_SUMMARY => {
            let lp: p::LimitParams = parse_params(params)?;
            let e = s.read();
            let a = e.analysis.clone();
            let mut ranked = a.files.clone();
            ranked.sort_by(|x, y| a.rank_of(*y).total_cmp(&a.rank_of(*x)));
            let top_load_bearing = ranked
                .into_iter()
                .take(lp.limit.min(200) as usize)
                .map(|f| p::RankedRef {
                    node: node_ref(e.node_ref(f)),
                    load: a.load_of(f) as f64 / 65535.0,
                    blast_count: e.blast_files(f).len() as u32,
                })
                .collect();
            to_value(p::SummaryResult {
                files: a.files.len() as u32,
                communities: a.community_sizes.iter().filter(|&&n| n > 1).count() as u32,
                file_cycles: a.file_cycles.len() as u32,
                dir_cycles: a.dir_cycles.len() as u32,
                dead: a.dead.len() as u32,
                analysis_ms: a.millis,
                top_load_bearing,
            })
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
    let (in_count, out_count) = e.edge_counts(slot);
    let mut ancestors = Vec::new();
    let mut cur = i.parent;
    while let Some(a) = cur {
        ancestors.push(a);
        let n = e.graph.node(a);
        cur = (n.parent != compass_core::model::NONE).then_some(n.parent);
    }
    let file_path = e.file_path_of(slot);
    let owner = file_path.and_then(|f| e.history.owner.get(f));
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
        rect: i.rect.map(rect_dto),
        ancestors,
        in_count,
        out_count,
        load: e.analysis.load_of(slot) as f64 / 65535.0,
        community: e.analysis.community_of(if i.kind.is_symbol() {
            e.graph.node(slot).file
        } else {
            slot
        }),
        blast_count: e.blast_files(slot).len() as u32,
        complexity: if i.kind.is_symbol() {
            e.complexity(e.graph.node(slot).file)
                .get(&slot)
                .copied()
                .unwrap_or(0)
        } else {
            0
        },
        dead_candidate: e.analysis.is_dead(slot),
        data: i.flags & flags::DATA != 0,
        area_lines: if i.kind == compass_core::model::NodeKind::File {
            compass_core::layout::area_lines(
                e.graph.node(slot),
                e.options
                    .cap_data_files
                    .then_some(compass_core::layout::DATA_CAP_LINES),
            )
        } else {
            i.loc
        },
        in_cycle: e.analysis.is_in_cycle(slot),
        churn: file_path
            .and_then(|f| e.history.churn.get(f).copied())
            .unwrap_or(0),
        owner: owner.map(|o| o.0.clone()),
        owner_share: owner.map(|o| o.1 as f64).unwrap_or(0.0),
    })
}

fn rect_dto(r: compass_core::Rect) -> p::Rect {
    p::Rect {
        x: r.x,
        y: r.y,
        w: r.w,
        h: r.h,
    }
}

fn node_ref(r: compass_engine::context::NodeRef) -> p::NodeRef {
    p::NodeRef {
        slot: r.slot,
        name: r.name,
        kind: kind_dto(r.kind),
        path: r.path,
        line: r.line,
    }
}

fn refs(v: Vec<compass_engine::context::NodeRef>) -> Vec<p::NodeRef> {
    v.into_iter().map(node_ref).collect()
}

fn cycle_view(e: &Engine, c: &compass_engine::analysis::Cycle) -> p::CycleView {
    p::CycleView {
        members: c.nodes.iter().map(|&s| node_ref(e.node_ref(s))).collect(),
        break_from: c.break_edge.map(|(a, _)| node_ref(e.node_ref(a))),
        break_to: c.break_edge.map(|(_, b)| node_ref(e.node_ref(b))),
    }
}

fn edge_kind_dto(k: compass_core::model::EdgeKind) -> p::EdgeKind {
    use compass_core::model::EdgeKind as K;
    match k {
        K::Contains => p::EdgeKind::Contains,
        K::Imports => p::EdgeKind::Imports,
        K::Calls => p::EdgeKind::Calls,
        K::References => p::EdgeKind::References,
        K::Inherits => p::EdgeKind::Inherits,
        K::Implements => p::EdgeKind::Implements,
        K::Cochanges => p::EdgeKind::Cochanges,
    }
}

fn confidence_dto(c: compass_core::model::Confidence) -> p::Confidence {
    use compass_core::model::Confidence as C;
    match c {
        C::Syntactic => p::Confidence::Syntactic,
        C::Semantic => p::Confidence::Semantic,
        C::Historical => p::Confidence::Historical,
    }
}

fn edges(e: &Engine, slot: u32, limit: usize) -> p::EdgesResult {
    let all = e.edges_of(slot, usize::MAX);
    let total = all.len() as u32;
    let edges = all
        .into_iter()
        .take(limit)
        .map(|v| {
            let n = e.graph.node(v.other);
            let file = if n.kind.is_symbol() { n.file } else { v.other };
            let name = if n.kind.is_symbol() {
                e.graph.qualified_name(v.other)
            } else {
                n.name.to_string()
            };
            let route = e
                .bundle_path(slot, v.other)
                .into_iter()
                .map(|r| p::Rect {
                    x: r.x,
                    y: r.y,
                    w: r.w,
                    h: r.h,
                })
                .collect();
            p::EdgeView {
                other: v.other,
                outgoing: v.outgoing,
                kind: edge_kind_dto(v.kind),
                confidence: confidence_dto(v.conf),
                weight: v.weight as f64,
                count: v.count,
                name,
                other_kind: kind_dto(n.kind),
                path: e.graph.path(file).unwrap_or("").to_string(),
                route,
            }
        })
        .collect();
    p::EdgesResult { slot, edges, total }
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
