// Keyboard map overlay (`?`).

export const KEYS: [string, string][] = [
  ["/", "Search symbols and files"],
  ["Enter", "Open selected in editor"],
  ["Arrow keys", "Move selection to a neighbouring tile (Map) or along edges (Neighbourhood)"],
  ["+  -  0", "Zoom in / out / fit"],
  ["1 – 6", "Switch lens"],
  ["M  N  P", "Map / Neighbourhood of selection / Path from pinned (or previous) to selection"],
  ["F", "Toggle follow mode"],
  ["Backspace", "Back in selection history"],
  [".", "Pin selection"],
  ["?", "Show this table"],
];

export class Help {
  readonly el: HTMLElement;
  constructor(parent: HTMLElement) {
    this.el = document.createElement("div");
    this.el.className = "help";
    this.el.hidden = true;
    this.el.setAttribute("role", "dialog");
    this.el.setAttribute("aria-label", "Keyboard shortcuts");
    const table = document.createElement("table");
    for (const [k, v] of KEYS) {
      const tr = document.createElement("tr");
      const a = document.createElement("td");
      a.className = "key";
      a.textContent = k;
      const b = document.createElement("td");
      b.textContent = v;
      tr.append(a, b);
      table.append(tr);
    }
    this.el.append(table);
    this.el.addEventListener("click", () => this.toggle(false));
    parent.append(this.el);
  }
  toggle(show = this.el.hidden): void {
    this.el.hidden = !show;
  }
}
