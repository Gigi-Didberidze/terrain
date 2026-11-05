// Request/response channel to the extension host, which forwards to the indexer.
import type { Requests } from "../../protocol/generated/protocol.ts";
import type { HostToView, ViewRequestMethod, ViewToHost } from "../../protocol/webview.ts";

declare function acquireVsCodeApi(): { postMessage(m: ViewToHost): void; getState(): unknown; setState(s: unknown): void };

export class Host {
  private readonly api = acquireVsCodeApi();
  private next = 1;
  private readonly pending = new Map<number, { resolve: (v: unknown) => void; reject: (e: Error) => void }>();
  onMessage: ((m: HostToView) => void) | undefined;

  constructor() {
    window.addEventListener("message", (ev: MessageEvent<HostToView>) => {
      const m = ev.data;
      if (m.type === "response") {
        const p = this.pending.get(m.id);
        if (!p) return;
        this.pending.delete(m.id);
        if (m.error !== undefined) p.reject(new Error(m.error));
        else p.resolve(m.result);
        return;
      }
      this.onMessage?.(m);
    });
  }

  post(m: ViewToHost): void {
    this.api.postMessage(m);
  }

  request<M extends ViewRequestMethod>(method: M, params: Requests[M]["params"]): Promise<Requests[M]["result"]> {
    const id = this.next++;
    return new Promise((resolve, reject) => {
      this.pending.set(id, { resolve: resolve as (v: unknown) => void, reject });
      this.api.postMessage({ type: "request", id, method, params });
    });
  }

  get state(): Record<string, unknown> {
    return (this.api.getState() as Record<string, unknown> | undefined) ?? {};
  }

  setState(patch: Record<string, unknown>): void {
    this.api.setState({ ...this.state, ...patch });
  }
}
