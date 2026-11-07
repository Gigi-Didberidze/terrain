// Screen-reader announcements. The map is a canvas, so what it shows is spoken through one
// polite live region: the selection, lens and view changes, and when the map is ready.
// Visually hidden; repeated text is re-announced by clearing first.

export class Announcer {
  readonly el: HTMLElement;
  private timer = 0;
  private last = "";

  constructor(parent: HTMLElement) {
    this.el = document.createElement("div");
    this.el.className = "sr-only";
    this.el.setAttribute("role", "status");
    this.el.setAttribute("aria-live", "polite");
    this.el.setAttribute("aria-atomic", "true");
    parent.append(this.el);
  }

  /** Speaks `text` after a short pause; a newer announcement replaces a pending one. */
  say(text: string): void {
    clearTimeout(this.timer);
    this.timer = window.setTimeout(() => {
      // Same text twice in a row is only spoken again if the region changes in between.
      if (text === this.last) this.el.textContent = "";
      this.last = text;
      window.setTimeout(() => (this.el.textContent = text), 20);
    }, 150);
  }
}
