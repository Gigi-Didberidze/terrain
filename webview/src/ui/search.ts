// Fuzzy search over files and symbols. One request per keystroke; stale responses are dropped.
import type { SearchHit } from "../../../protocol/generated/protocol.ts";

export class SearchBox {
  readonly el: HTMLElement;
  private readonly input: HTMLInputElement;
  private readonly list: HTMLElement;
  private hits: SearchHit[] = [];
  private active = 0;
  private seq = 0;
  private readonly query: (q: string) => Promise<{ hits: SearchHit[]; elapsedMs: number }>;
  onPick: ((hit: SearchHit) => void) | undefined;
  onClose: (() => void) | undefined;

  constructor(parent: HTMLElement, query: (q: string) => Promise<{ hits: SearchHit[]; elapsedMs: number }>) {
    this.query = query;
    this.el = document.createElement("div");
    this.el.className = "search";
    this.el.hidden = true;
    this.input = document.createElement("input");
    this.input.type = "text";
    this.input.spellcheck = false;
    this.input.placeholder = "Search files and symbols";
    this.input.setAttribute("aria-label", "Search files and symbols");
    this.input.setAttribute("role", "combobox");
    this.input.setAttribute("aria-autocomplete", "list");
    this.input.setAttribute("aria-controls", "terrain-search-list");
    this.input.setAttribute("aria-expanded", "false");
    this.list = document.createElement("div");
    this.list.className = "search-list";
    this.list.id = "terrain-search-list";
    this.list.setAttribute("role", "listbox");
    this.list.setAttribute("aria-label", "Matches");
    this.el.append(this.input, this.list);
    parent.append(this.el);
    this.input.addEventListener("input", () => void this.run());
    this.input.addEventListener("keydown", (e) => this.onKey(e));
  }

  get open(): boolean {
    return !this.el.hidden;
  }

  show(): void {
    this.el.hidden = false;
    this.input.focus();
    this.input.select();
    if (this.input.value) void this.run();
  }

  hide(): void {
    this.el.hidden = true;
    this.onClose?.();
  }

  private async run(): Promise<void> {
    const q = this.input.value;
    const seq = ++this.seq;
    if (!q.trim()) {
      this.hits = [];
      this.render();
      return;
    }
    try {
      const r = await this.query(q);
      if (seq !== this.seq) return;
      this.hits = r.hits;
      this.active = 0;
      this.render();
    } catch {
      // Indexer busy or restarting; the next keystroke retries.
    }
  }

  private render(): void {
    this.list.replaceChildren();
    this.input.setAttribute("aria-expanded", String(this.hits.length > 0));
    this.input.removeAttribute("aria-activedescendant");
    if (!this.hits.length && this.input.value.trim()) {
      const none = document.createElement("div");
      none.className = "search-row muted";
      none.setAttribute("role", "option");
      none.setAttribute("aria-disabled", "true");
      none.textContent = "No files or symbols match";
      this.list.append(none);
      return;
    }
    this.hits.forEach((h, i) => {
      const row = document.createElement("div");
      row.className = "search-row";
      row.id = `terrain-search-${i}`;
      row.setAttribute("role", "option");
      row.setAttribute("aria-selected", String(i === this.active));
      row.setAttribute("aria-label", `${h.kind} ${h.name}, ${h.kind === "file" ? h.path : `${h.path} line ${h.line}`}`);
      if (i === this.active) this.input.setAttribute("aria-activedescendant", row.id);
      const name = document.createElement("span");
      name.textContent = h.name;
      const kind = document.createElement("span");
      kind.className = "muted";
      kind.textContent = ` ${h.kind}`;
      const path = document.createElement("span");
      path.className = "muted search-path";
      path.textContent = h.kind === "file" ? h.path : `${h.path}:${h.line}`;
      row.append(name, kind, path);
      row.addEventListener("mousedown", (e) => {
        e.preventDefault();
        this.pick(i);
      });
      this.list.append(row);
    });
  }

  private pick(i: number): void {
    const h = this.hits[i];
    if (!h) return;
    this.hide();
    this.onPick?.(h);
  }

  private onKey(e: KeyboardEvent): void {
    e.stopPropagation();
    if (e.key === "Escape") this.hide();
    else if (e.key === "Enter") this.pick(this.active);
    else if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      const n = this.hits.length;
      if (!n) return;
      this.active = (this.active + (e.key === "ArrowDown" ? 1 : n - 1)) % n;
      this.render();
    }
  }
}
