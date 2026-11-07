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
  | "history/file";
export const VIEW_REQUEST_METHODS: readonly ViewRequestMethod[] = [
  "map/info",
  "map/tiles",
  "node/info",
  "node/locate",
  "node/edges",
  "search",
  "history/summary",
  "history/file",
];

export type HostToView =
  | { type: "status"; state: "starting" | "ready" | "error"; text: string }
  | { type: "progress"; phase: string; done: number; total: number }
  | { type: "info"; info: MapInfo }
  | { type: "changed"; epoch: number; layout: boolean }
  /** Response to a ViewToHost request. Tile data arrives as raw bytes (`Uint8Array`), not base64. */
  | { type: "response"; id: number; result?: unknown; error?: string }
  /** The active editor moved: the "you are here" node (null when the file is not on the map). */
  | { type: "follow"; node: NodeInfo | null }
  | { type: "reveal"; slot: number }
  | { type: "command"; command: "fit" | "toggleFollow" | "search" };

export type ViewToHost =
  | { type: "ready" }
  | { type: "request"; id: number; method: ViewRequestMethod; params: Requests[ViewRequestMethod]["params"] }
  | { type: "open"; slot: number }
  | { type: "select"; slot: number | null }
  | { type: "follow"; enabled: boolean }
  | { type: "log"; level: "info" | "error"; text: string };

/** Tile as delivered to the webview. */
export interface ViewTile {
  z: number;
  x: number;
  y: number;
  data: Uint8Array;
}
