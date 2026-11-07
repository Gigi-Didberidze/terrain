// Reads colours from VS Code theme tokens. Compass defines no colours of its own: every fill,
// stroke and label comes from the active theme, so it looks native in light, dark and high contrast.

export type RGBA = [number, number, number, number];

export interface Theme {
  kind: "light" | "dark" | "high-contrast" | "high-contrast-light";
  background: RGBA;
  foreground: RGBA;
  muted: RGBA;
  line: RGBA;
  accent: RGBA;
  dirFill: RGBA;
  fileFill: RGBA;
  symbolFill: RGBA;
  generatedFill: RGBA;
  /** Categorical palette (8) for communities and owners. */
  categorical: RGBA[];
  /** Ramp end for sequential lenses. */
  rampHigh: RGBA;
  /** Churn lens ramp end. */
  churnHigh: RGBA;
  /** Coupling lens: partners with a code link, and hidden dependencies (no code link). */
  coupling: RGBA;
  hiddenCoupling: RGBA;
  /** Change lens: changed files (strong) and their blast radius (light tint of the same hue). */
  changed: RGBA;
  editorFont: string;
  uiFont: string;
}

let probe: HTMLElement | undefined;

/** Resolves any CSS colour (including var(--token)) to RGBA 0..255 via the browser. */
export function cssColor(value: string, fallback: string): RGBA {
  if (!probe) {
    probe = document.createElement("div");
    probe.style.display = "none";
    document.body.append(probe);
  }
  probe.style.color = "";
  probe.style.color = value;
  if (!probe.style.color) probe.style.color = fallback;
  const c = getComputedStyle(probe).color;
  const m = /rgba?\(([^)]+)\)/.exec(c);
  if (!m) return [0, 0, 0, 255];
  const parts = m[1]!.split(/[\s,/]+/).filter(Boolean).map(Number);
  return [parts[0] ?? 0, parts[1] ?? 0, parts[2] ?? 0, Math.round((parts[3] ?? 1) * 255)];
}

/** WCAG 2 relative luminance. */
export function luminance([r, g, b]: RGBA): number {
  const ch = (v: number) => {
    const s = v / 255;
    return s <= 0.03928 ? s / 12.92 : ((s + 0.055) / 1.055) ** 2.4;
  };
  return 0.2126 * ch(r) + 0.7152 * ch(g) + 0.0722 * ch(b);
}

export function contrast(a: RGBA, b: RGBA): number {
  const [x, y] = [luminance(a), luminance(b)].sort((p, q) => q - p);
  return (x! + 0.05) / (y! + 0.05);
}

export function mix(a: RGBA, b: RGBA, t: number): RGBA {
  return [
    Math.round(a[0] + (b[0] - a[0]) * t),
    Math.round(a[1] + (b[1] - a[1]) * t),
    Math.round(a[2] + (b[2] - a[2]) * t),
    Math.round(a[3] + (b[3] - a[3]) * t),
  ];
}

/** Perceptual ramp through OKLab between two colours. */
export function ramp(a: RGBA, b: RGBA, t: number): RGBA {
  const la = toOklab(a);
  const lb = toOklab(b);
  const l: [number, number, number] = [la[0] + (lb[0] - la[0]) * t, la[1] + (lb[1] - la[1]) * t, la[2] + (lb[2] - la[2]) * t];
  const [r, g, bl] = fromOklab(l);
  return [r, g, bl, 255];
}

function srgbToLinear(c: number): number {
  c /= 255;
  return c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
}
function linearToSrgb(c: number): number {
  const v = c <= 0.0031308 ? 12.92 * c : 1.055 * c ** (1 / 2.4) - 0.055;
  return Math.round(Math.min(1, Math.max(0, v)) * 255);
}
function toOklab([r8, g8, b8]: RGBA): [number, number, number] {
  const r = srgbToLinear(r8);
  const g = srgbToLinear(g8);
  const b = srgbToLinear(b8);
  const l = Math.cbrt(0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * b);
  const m = Math.cbrt(0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * b);
  const s = Math.cbrt(0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * b);
  return [
    0.2104542553 * l + 0.793617785 * m - 0.0040720468 * s,
    1.9779984951 * l - 2.428592205 * m + 0.4505937099 * s,
    0.0259040371 * l + 0.7827717662 * m - 0.808675766 * s,
  ];
}
function fromOklab([L, A, B]: [number, number, number]): [number, number, number] {
  const l = (L + 0.3963377774 * A + 0.2158037573 * B) ** 3;
  const m = (L - 0.1055613458 * A - 0.0638541728 * B) ** 3;
  const s = (L - 0.0894841775 * A - 1.291485548 * B) ** 3;
  return [
    linearToSrgb(4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s),
    linearToSrgb(-1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s),
    linearToSrgb(-0.0041960863 * l - 0.7034186147 * m + 1.707614701 * s),
  ];
}

function themeKind(): Theme["kind"] {
  const c = document.body.classList;
  if (c.contains("vscode-high-contrast-light")) return "high-contrast-light";
  if (c.contains("vscode-high-contrast")) return "high-contrast";
  if (c.contains("vscode-light")) return "light";
  return "dark";
}

export function readTheme(): Theme {
  const kind = themeKind();
  const bg = cssColor("var(--vscode-editor-background)", "#1e1e1e");
  const fg = cssColor("var(--vscode-editor-foreground)", "#cccccc");
  const hc = kind.startsWith("high-contrast");
  // Strokes are derived from background and foreground so they keep the same contrast in every
  // theme; widget borders are often nearly invisible on dark themes.
  const line = hc ? cssColor("var(--vscode-contrastBorder)", "#6fc3df") : mix(bg, fg, kind === "light" ? 0.2 : 0.22);
  const cs = getComputedStyle(document.body);
  const chart = (name: string, fb: string) => cssColor(`var(--vscode-charts-${name})`, fb);
  return {
    kind,
    background: bg,
    foreground: fg,
    muted: cssColor("var(--vscode-descriptionForeground)", "#9d9d9d"),
    line,
    accent: cssColor("var(--vscode-focusBorder)", "#007fd4"),
    dirFill: hc ? bg : mix(bg, fg, 0.025),
    fileFill: hc ? bg : mix(bg, fg, 0.07),
    symbolFill: hc ? bg : mix(bg, fg, 0.12),
    generatedFill: hc ? bg : mix(bg, fg, 0.035),
    categorical: [
      chart("blue", "#3794ff"),
      chart("orange", "#d18616"),
      chart("green", "#89d185"),
      chart("purple", "#b180d7"),
      chart("red", "#f14c4c"),
      chart("yellow", "#cca700"),
      cssColor("var(--vscode-terminal-ansiCyan)", "#11a8cd"),
      cssColor("var(--vscode-terminal-ansiMagenta)", "#bc3fbc"),
    ],
    rampHigh: chart("blue", "#3794ff"),
    churnHigh: chart("orange", "#d18616"),
    coupling: chart("yellow", "#cca700"),
    hiddenCoupling: chart("red", "#f14c4c"),
    changed: chart("orange", "#d18616"),
    editorFont: cs.getPropertyValue("--vscode-editor-font-family").trim() || "monospace",
    uiFont: cs.getPropertyValue("--vscode-font-family").trim() || "sans-serif",
  };
}
