// Lenses colour the map by exactly one measure at a time (keys 1–6). Colours come from theme
// tokens; the legend always names the measure and its unit.
import { mix, ramp, type RGBA, type Theme } from "./render/theme.ts";
import { KIND_DIR, KIND_FILE, type Item } from "./tiles.ts";

export type LensId = 1 | 2 | 3 | 4 | 5 | 6;

export interface LensDef {
  id: LensId;
  name: string;
  /** One line: what colour means, with the unit. */
  legend: string;
  /** Milestone that brings it, when not available yet. */
  pending?: string;
}

export const LENSES: Record<LensId, LensDef> = {
  1: { id: 1, name: "Structure", legend: "Structure: colour = module found from dependencies (8 largest), grey = other" },
  2: { id: 2, name: "Load-bearing", legend: "Load-bearing: stronger = more depends on it (PageRank, log scale)" },
  3: { id: 3, name: "Churn", legend: "Churn: commits in the last 90 days", pending: "git history (M4)" },
  4: { id: 4, name: "Coupling", legend: "Coupling: hidden co-change partners of the selection", pending: "git history (M4)" },
  5: { id: 5, name: "Ownership", legend: "Ownership: top author by lines", pending: "git history (M4)" },
  6: { id: 6, name: "Change", legend: "Change: files changed on this branch and their blast radius", pending: "review mode (M5)" },
};

/** Fill for an item under a lens, or null for the neutral fill. Directories are never coloured. */
export function lensColor(lens: LensId, it: Item, theme: Theme): RGBA | null {
  if (it.kind === KIND_DIR) return null;
  const base = it.kind === KIND_FILE ? theme.fileFill : theme.symbolFill;
  switch (lens) {
    case 1: {
      if (!it.community) return null;
      const c = theme.categorical[(it.community - 1) % theme.categorical.length]!;
      // Muted towards the background so labels stay readable; symbols a little stronger.
      return mix(base, c, it.kind === KIND_FILE ? 0.45 : 0.6);
    }
    case 2: {
      // Log-scaled load (see analysis.rs log_scale); slight easing keeps the middle calm.
      const t = Math.pow(it.load / 65535, 1.6);
      return ramp(base, theme.rampHigh, Math.min(1, t * 0.9));
    }
    default:
      return null;
  }
}
