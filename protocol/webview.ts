// Messages between the extension host and the map webview (hand-written; the indexer
// protocol is generated separately in generated/protocol.ts).
import type { MapInfo, NodeInfo, Requests } from "./generated/protocol.ts";

/** Indexer methods the webview may call through the host. */
export type ViewRequestMethod =
  | "map/info"
  | "map/tiles"
  | "node/info"
  | "node/locate"
  | "node/edges"
  | "search"
  | "history/summary"
  | "history/file"
  | "view/neighbourhood"
  | "view/path"
  | "review/summary";
export const VIEW_REQUEST_METHODS: readonly ViewRequestMethod[] = [
  "map/info",
  "map/tiles",
  "node/info",
  "node/locate",
  "node/edges",
  "search",
  "history/summary",
  "history/file",
  "view/neighbourhood",
  "view/path",
  "review/summary",
];

/** Commands a state screen may offer; the host runs only these. */
export type StateCommand = "compass.restartIndexer" | "compass.showLog" | "compass.reindex" | "workbench.action.files.openFolder";
export const STATE_COMMANDS: readonly StateCommand[] = ["compass.restartIndexer", "compass.showLog", "compass.reindex", "workbench.action.files.openFolder"];

export interface StateAction {
  label: string;
  command: StateCommand;
}

export type HostToView =
  /**
   * Whole-view states. "starting" and "ready" are transient; "empty" (nothing to map, no folder)
   * and "error" replace the map with a title, one line of explanation and actions.
   */
  | { type: "status"; state: "starting" | "ready" | "error" | "empty"; text: string; detail?: string; actions?: StateAction[] }
  | { type: "progress"; phase: string; done: number; total: number }
  | { type: "info"; info: MapInfo }
  | { type: "changed"; epoch: number; layout: boolean }
  /** Response to a ViewToHost request. Tile data arrives as raw bytes (`Uint8Array`), not base64. */
  | { type: "response"; id: number; result?: unknown; error?: string }
  /** The active editor moved: the "you are here" node (null when the file is not on the map). */
  | { type: "follow"; node: NodeInfo | null }
  | { type: "reveal"; slot: number }
  /** Settings the view needs; sent when the panel opens. */
  | { type: "config"; followDefault: boolean }
  | { type: "command"; command: "fit" | "toggleFollow" | "search" | "map" | "exportSvg" }
  | { type: "command"; command: "neighbourhood"; slot: number }
  | { type: "command"; command: "path"; from: number; to: number }
  | { type: "command"; command: "lens"; lens: number };

export type ViewToHost =
  | { type: "ready" }
  /** A button on a state screen. */
  | { type: "action"; command: StateCommand }
  | { type: "request"; id: number; method: ViewRequestMethod; params: Requests[ViewRequestMethod]["params"] }
  | { type: "open"; slot: number }
  | { type: "select"; slot: number | null }
  | { type: "follow"; enabled: boolean }
  | { type: "log"; level: "info" | "error"; text: string }
  /** Answer to the exportSvg command: the current view as an SVG document. */
  | { type: "svg"; svg: string };

/** Tile as delivered to the webview. */
export interface ViewTile {
  z: number;
  x: number;
  y: number;
  data: Uint8Array;
}
