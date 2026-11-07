// Lenses colour the map by exactly one measure at a time (keys 1–6). Colours come from theme
// tokens; the legend always names the measure and its unit.
import { contrast, mix, ramp, type RGBA, type Theme } from "./render/theme.ts";
import { FLAG_BLAST, FLAG_CHANGED, KIND_DIR, KIND_FILE, type Item } from "./tiles.ts";

export type LensId = 1 | 2 | 3 | 4 | 5 | 6;

export interface LensDef {
  id: LensId;
  name: string;
  /** One line: what colour means, with the unit. */
  legend: string;
  /** Needs git history (lenses 3–5 are unavailable outside a repository). */
  history?: boolean;
  /** Needs a base branch or working-tree changes (lens 6). */
  review?: boolean;
  /** Milestone that brings it, when not available yet. */
  pending?: string;
}

export const LENSES: Record<LensId, LensDef> = {
  1: { id: 1, name: "Structure", legend: "Structure: colour = module found from dependencies and co-change (8 largest), grey = other" },
  2: { id: 2, name: "Load-bearing", legend: "Load-bearing: stronger = more depends on it (PageRank, log scale)" },
  3: { id: 3, name: "Churn", legend: "Churn: stronger = more commits in the last 90 days (log scale)", history: true },
  4: { id: 4, name: "Coupling", legend: "Coupling: files that change with the selection; red = no code link (hidden dependency)", history: true },
  5: { id: 5, name: "Ownership", legend: "Ownership: top author by commits", history: true },
  6: { id: 6, name: "Change", legend: "Change: changed on this branch (strong), depends on a change (light)", review: true },
};

/** A co-change partner of the selected file (Coupling lens). */
export interface CouplingPartner {
  slot: number;
  degree: number;
  hidden: boolean;
  rect?: { x: number; y: number; w: number; h: number };
}

/** The selected file and its co-change partners. */
export interface CouplingFocus {
  file: number;
  fileRect?: { x: number; y: number; w: number; h: number };
  partners: Map<number, CouplingPartner>;
}

/** Fill for an item under a lens, or null for the neutral fill. Directories are never coloured. */
export function lensColor(lens: LensId, it: Item, theme: Theme, coupling?: CouplingFocus | null): RGBA | null {
  const c = rawLensColor(lens, it, theme, coupling);
  return c && legible(c, theme);
}

/** Labels need 4.5:1 against their fill (WCAG AA). */
const MIN_CONTRAST = 4.5;
const legibleCache = new WeakMap<Theme, Map<number, RGBA>>();

/**
 * Keeps a lens colour if either the theme's text or its background colour reaches 4.5:1 on it
 * (labels pick that one, see textOn); otherwise pulls it toward the background until the text
 * colour does. Mid-tones are the only casualties; vivid and deep colours pass unchanged.
 * Checked in three themes by test-visual.
 */
export function legible(c: RGBA, theme: Theme): RGBA {
  let cache = legibleCache.get(theme);
  if (!cache) legibleCache.set(theme, (cache = new Map()));
  const key = (c[0] << 16) | (c[1] << 8) | c[2];
  const hit = cache.get(key);
  if (hit) return hit;
  let out = c;
  if (Math.max(contrast(theme.foreground, c), contrast(theme.background, c)) < MIN_CONTRAST) {
    for (let i = 1; i <= 20 && contrast(theme.foreground, out) < MIN_CONTRAST; i++) out = mix(c, theme.background, i / 20);
  }
  cache.set(key, out);
  return out;
}

/** Label colour on a fill: the theme's text colour, or its background colour where that reads better. */
export function textOn(fill: RGBA, theme: Theme): RGBA {
  return contrast(theme.foreground, fill) >= contrast(theme.background, fill) ? theme.foreground : theme.background;
}

function rawLensColor(lens: LensId, it: Item, theme: Theme, coupling?: CouplingFocus | null): RGBA | null {
  if (it.kind === KIND_DIR) return null;
  const isFile = it.kind === KIND_FILE;
  const base = isFile ? theme.fileFill : theme.symbolFill;
  switch (lens) {
    case 1: {
      if (!it.community) return null;
      const c = theme.categorical[(it.community - 1) % theme.categorical.length]!;
      // Muted towards the background so labels stay readable; symbols a little stronger.
      return mix(base, c, isFile ? 0.45 : 0.6);
    }
    case 2: {
      // Log-scaled load (see analysis.rs log_scale); slight easing keeps the middle calm.
      const t = Math.pow(it.load / 65535, 1.6);
      return ramp(base, theme.rampHigh, Math.min(1, t * 0.9));
    }
    case 3: {
      if (!it.churn) return null;
      // Log-scaled against the busiest file (history.rs churn_lens); a floor keeps one commit visible.
      return ramp(base, theme.churnHigh, 0.12 + 0.78 * (it.churn / 65535));
    }
    case 4: {
      if (!coupling) return null;
      const p = partnerOf(it, coupling);
      if (!p) return null;
      const c = p.hidden ? theme.hiddenCoupling : theme.coupling;
      return mix(base, c, (isFile ? 0.25 : 0.35) + 0.5 * Math.min(1, p.degree));
    }
    case 6: {
      if (it.flags & FLAG_CHANGED) return mix(base, theme.changed, isFile ? 0.65 : 0.75);
      if (it.flags & FLAG_BLAST) return mix(base, theme.changed, isFile ? 0.2 : 0.25);
      return null;
    }
    case 5: {
      if (!it.owner) return null;
      const c = theme.categorical[(it.owner - 1) % theme.categorical.length]!;
      return mix(base, c, isFile ? 0.5 : 0.6);
    }
    default:
      return null;
  }
}

/** Coupling lens: everything outside the selected file and its partners is muted. */
export function couplingMuted(it: Item, coupling: CouplingFocus | null | undefined): boolean {
  if (!coupling || it.kind === KIND_DIR) return false;
  if (it.slot === coupling.file || inside(it, coupling.fileRect)) return false;
  return !partnerOf(it, coupling);
}

function partnerOf(it: Item, c: CouplingFocus): CouplingPartner | undefined {
  const direct = c.partners.get(it.slot);
  if (direct || it.kind === KIND_FILE) return direct;
  // Symbols take their file's partner status; tiles do not carry the file, so go by geometry.
  for (const p of c.partners.values()) if (inside(it, p.rect)) return p;
  return undefined;
}

function inside(it: Item, r: { x: number; y: number; w: number; h: number } | undefined): boolean {
  if (!r) return false;
  const cx = it.x + it.w / 2;
  const cy = it.y + it.h / 2;
  return cx >= r.x && cx <= r.x + r.w && cy >= r.y && cy <= r.y + r.h;
}

/** "ann@example.com" → "ann", "123+ann@users.noreply.github.com" → "ann"; names stay as they are. */
export function shortAuthor(a: string): string {
  const at = a.indexOf("@");
  const local = at > 0 ? a.slice(0, at) : a;
  return local.replace(/^\d+\+/, "");
}
