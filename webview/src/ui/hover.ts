// Hover card: no more than five lines.

export interface HoverLines {
  title: string;
  lines: string[];
}

export class HoverCard {
  readonly el: HTMLElement;
  constructor(parent: HTMLElement) {
    this.el = document.createElement("div");
    this.el.className = "hover";
    this.el.hidden = true;
    parent.append(this.el);
  }

  show(content: HoverLines, x: number, y: number): void {
    this.el.replaceChildren();
    const t = document.createElement("div");
    t.className = "hover-title";
    t.textContent = content.title;
    this.el.append(t);
    for (const l of content.lines.slice(0, 4)) {
      const d = document.createElement("div");
      d.textContent = l;
      this.el.append(d);
    }
    this.el.hidden = false;
    const r = this.el.getBoundingClientRect();
    const W = window.innerWidth;
    const H = window.innerHeight;
    const left = x + 14 + r.width > W ? x - 14 - r.width : x + 14;
    const top = y + 14 + r.height > H ? y - 14 - r.height : y + 14;
    this.el.style.left = `${Math.max(4, left)}px`;
    this.el.style.top = `${Math.max(4, top)}px`;
  }

  hide(): void {
    this.el.hidden = true;
  }
}
