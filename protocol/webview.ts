// Messages between the extension host and the map webview (hand-written; the indexer
// protocol is generated separately in generated/protocol.ts).

export type HostToView =
  | { type: "status"; state: "starting" | "ready" | "error"; text: string }
  | { type: "progress"; phase: string; done: number; total: number };

export type ViewToHost =
  | { type: "ready" }
  | { type: "log"; level: "info" | "error"; text: string };
