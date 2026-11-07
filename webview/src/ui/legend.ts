// The legend: always visible, always states the measure and its unit, never more than one line.

export class Legend {
  readonly el: HTMLElement;
  private readonly measure: HTMLElement;
  private readonly count: HTMLButtonElement;
  private readonly status: HTMLElement;
  private readonly keys: HTMLElement;
  onCountClick: (() => void) | undefined;

  constructor(parent: HTMLElement) {
    this.el = document.createElement("div");
    this.el.className = "legend";
    // Not a live region: it changes with every progress tick. The Announcer speaks what matters.
    this.el.setAttribute("aria-label", "Legend");
    this.measure = document.createElement("span");
    this.count = document.createElement("button");
    this.count.className = "link";
    this.count.title = "Search files";
    this.count.addEventListener("click", () => this.onCountClick?.());
    this.status = document.createElement("span");
    this.status.className = "muted";
    this.keys = document.createElement("span");
    this.keys.className = "keys";
    this.el.append(this.measure, this.keys, sep(), this.count, this.status);
    parent.append(this.el);
  }

  /** `swatches`: a colour key for categorical lenses (Ownership lists its authors). */
  set(measure: string, files: number, status: string, swatches: { color: string; label: string }[] = []): void {
    this.measure.textContent = measure;
    this.keys.replaceChildren(
      ...swatches.map((s) => {
        const k = document.createElement("span");
        k.className = "key";
        const dot = document.createElement("span");
        dot.className = "swatch";
        dot.style.background = s.color;
        k.append(dot, s.label);
        return k;
      }),
    );
    this.count.textContent = `${files.toLocaleString("en-US")} files`;
    this.status.textContent = status ? ` · ${status}` : "";
  }
}

function sep(): HTMLElement {
  const s = document.createElement("span");
  s.className = "muted";
  s.textContent = " · ";
  return s;
}
