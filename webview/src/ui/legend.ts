// The legend: always visible, always states the measure and its unit, never more than one line.

export class Legend {
  readonly el: HTMLElement;
  private readonly measure: HTMLElement;
  private readonly count: HTMLButtonElement;
  private readonly status: HTMLElement;
  onCountClick: (() => void) | undefined;

  constructor(parent: HTMLElement) {
    this.el = document.createElement("div");
    this.el.className = "legend";
    this.el.setAttribute("role", "status");
    this.measure = document.createElement("span");
    this.count = document.createElement("button");
    this.count.className = "link";
    this.count.title = "Search files";
    this.count.addEventListener("click", () => this.onCountClick?.());
    this.status = document.createElement("span");
    this.status.className = "muted";
    this.el.append(this.measure, sep(), this.count, this.status);
    parent.append(this.el);
  }

  set(measure: string, files: number, status: string): void {
    this.measure.textContent = measure;
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
