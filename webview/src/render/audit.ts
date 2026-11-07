// Contrast audit: every label colour against every fill it can be drawn on, in the active theme
// and under every lens. Used by the visual tests in three themes and runnable in DevTools
// (window.__terrainApp.themeAudit()). WCAG 2 contrast; labels need 4.5:1 (AA, normal text).
import { LENSES, lensColor, textOn, type LensId } from "../lenses.ts";
import { FLAG_BLAST, FLAG_CHANGED, KIND_FILE, type Item } from "../tiles.ts";
import { contrast, type RGBA, type Theme } from "./theme.ts";

export interface ContrastCheck {
  what: string;
  ratio: number;
  min: number;
  ok: boolean;
}

const KIND_FUNCTION = 4;

export function auditTheme(theme: Theme): ContrastCheck[] {
  const out: ContrastCheck[] = [];
  const check = (what: string, fg: RGBA, bg: RGBA, min = 4.5) => {
    const ratio = contrast(fg, bg);
    out.push({ what, ratio: Math.round(ratio * 100) / 100, min, ok: ratio >= min });
  };
  check("directory label on directory fill", theme.muted, theme.dirFill);
  check("file label on file fill", theme.foreground, theme.fileFill);
  check("file label on generated/data fill", theme.foreground, theme.generatedFill);
  check("symbol label on symbol fill", theme.foreground, theme.symbolFill);
  check("legend text", theme.foreground, theme.background);
  // Lens fills: the strongest value each lens can produce, for files and symbols.
  const item = (kind: number, extra: Partial<Item>): Item => ({
    slot: 1, x: 0, y: 0, w: 1, h: 1, kind, depth: 0, flags: 0, hidden: 0, label: "", community: 0, owner: 0, load: 0, churn: 0, ...extra,
  });
  const strongest: Partial<Item>[] = [
    ...Array.from({ length: 8 }, (_, i) => ({ community: i + 1, owner: i + 1 })),
    { load: 65535, churn: 65535, flags: FLAG_CHANGED },
    { flags: FLAG_BLAST },
  ];
  for (const id of [1, 2, 3, 5, 6] as LensId[]) {
    for (const kind of [KIND_FILE, KIND_FUNCTION]) {
      let worst: { ratio: number; fill: RGBA } | undefined;
      for (const extra of strongest) {
        const fill = lensColor(id, item(kind, extra), theme);
        if (!fill) continue;
        const ratio = contrast(textOn(fill, theme), fill);
        if (!worst || ratio < worst.ratio) worst = { ratio, fill };
      }
      if (worst) check(`${kind === KIND_FILE ? "file" : "symbol"} label under the ${LENSES[id].name} lens (worst colour)`, textOn(worst.fill, theme), worst.fill);
    }
  }
  // Coupling lens: partner fills at full degree.
  const focus = { file: 99, partners: new Map([[1, { slot: 1, degree: 1, hidden: true }], [2, { slot: 2, degree: 1, hidden: false }]]) };
  for (const slot of [1, 2]) {
    const fill = lensColor(4, item(KIND_FILE, { slot }), theme, focus);
    if (fill) check(`file label under the Coupling lens (${slot === 1 ? "hidden" : "linked"} partner)`, textOn(fill, theme), fill);
  }
  // Non-text: the selection outline must stand out from what it outlines (3:1, WCAG 1.4.11).
  check("selection outline against file fill", theme.accent, theme.fileFill, 3);
  return out;
}
