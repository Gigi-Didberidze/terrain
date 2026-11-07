// Visual tests in three themes (dark, light, high contrast), run in CI on every push.
//
// Pixel-exact comparisons break across operating systems, GPUs and font hinting, so these test
// what makes the map readable instead:
//   - every label colour meets WCAG AA contrast against every fill it can sit on, under every lens
//   - the map draws (most of the view is tiles, not background) with no page errors
//   - the selection is outlined in the theme's accent colour
//   - the Neighbourhood view and the empty/error screens render
// Screenshots are written to the output folder for people to look at.
//
// Usage: node webview/test-visual/themes.mjs <workspace> [outDir] [--binary path]
import { chromium } from "playwright";
import { spawn } from "node:child_process";
import { mkdirSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const args = process.argv.slice(2);
const bi = args.indexOf("--binary");
const binary = bi >= 0 ? args.splice(bi, 2)[1] : undefined;
const [workspace, out = join(here, "out")] = args;
if (!workspace) {
  console.error("usage: node webview/test-visual/themes.mjs <workspace> [outDir] [--binary path]");
  process.exit(2);
}
mkdirSync(out, { recursive: true });

const port = 4790 + Math.floor(Math.random() * 100);
const server = spawn(process.execPath, [join(here, "..", "harness", "server.mjs"), workspace, "--port", String(port), ...(binary ? ["--binary", binary] : [])], {
  stdio: ["ignore", "pipe", "pipe"],
});
await new Promise((resolve, reject) => {
  server.stdout.on("data", (d) => String(d).includes("http://") && resolve());
  server.on("exit", (c) => reject(new Error(`harness exited (${c})`)));
  setTimeout(resolve, 5000);
});

const failures = [];
const fail = (msg) => {
  failures.push(msg);
  console.log(`  FAIL ${msg}`);
};
const browser = await chromium.launch({ args: ["--use-angle=swiftshader", "--enable-unsafe-swiftshader"] });
try {
  for (const theme of ["dark", "light", "hc"]) {
    console.log(`${theme}:`);
    const page = await browser.newPage({ viewport: { width: 1280, height: 800 } });
    const errors = [];
    page.on("pageerror", (e) => errors.push(e.message));
    page.on("console", (m) => m.type() === "error" && errors.push(m.text()));
    await page.goto(`http://localhost:${port}/?theme=${theme}`);
    await page.evaluate(() => sessionStorage.clear());
    await page.goto(`http://localhost:${port}/?theme=${theme}`);
    await page.waitForFunction(
      async () => {
        const r = await window.__terrain.rpc("index/status", null);
        return r.result && r.result.phase === "idle" && r.result.timings.totalMs > 0;
      },
      null,
      { timeout: 180000, polling: 500 },
    );
    await page.waitForTimeout(1500);

    // 1. Contrast of every label against every fill.
    const audit = await page.evaluate(() => window.__terrainApp.themeAudit());
    for (const c of audit) {
      if (!c.ok) fail(`${theme}: ${c.what}: contrast ${c.ratio} < ${c.min}`);
    }
    console.log(`  contrast: ${audit.filter((c) => c.ok).length}/${audit.length} pairs pass (lowest ${Math.min(...audit.map((c) => c.ratio)).toFixed(2)})`);

    // 2. The map draws.
    const shot = await page.screenshot({ path: join(out, `map-${theme}.png`) });
    const colors = await page.evaluate(() => window.__terrainApp.themeColors());
    const coverage = await pixels(page, shot, colors.background, 10);
    if (coverage < 0.5) fail(`${theme}: only ${(coverage * 100).toFixed(0)}% of the view is drawn`);
    console.log(`  map: ${(coverage * 100).toFixed(0)}% of the view drawn`);

    // 3. Selection outlined in the accent colour.
    await page.evaluate(async () => {
      const r = await window.__terrain.rpc("search", { query: "app", limit: 1 });
      const hit = r.result.hits[0];
      if (hit) await window.__terrainApp.reveal(hit.slot, true);
    });
    await page.waitForTimeout(800);
    const sel = await page.screenshot({ path: join(out, `selected-${theme}.png`) });
    const accent = await pixels(page, sel, colors.accent, 24, true);
    if (accent * 1280 * 800 < 200) fail(`${theme}: selection outline not drawn in the accent colour (${Math.round(accent * 1280 * 800)} px)`);
    console.log(`  selection: ${Math.round(accent * 1280 * 800)} accent pixels`);

    // 4. Neighbourhood view.
    await page.keyboard.press("n");
    await page.waitForTimeout(800);
    const nb = await page.screenshot({ path: join(out, `neighbourhood-${theme}.png`) });
    const nbCover = await pixels(page, nb, colors.background, 10);
    if (nbCover < 0.01) fail(`${theme}: Neighbourhood view is empty`);
    await page.keyboard.press("m");

    // 5. State screens.
    await page.evaluate(() =>
      window.postMessage(
        {
          type: "status",
          state: "error",
          text: "The indexer keeps stopping",
          detail: "It exited with code 3 twice within a minute.",
          actions: [
            { label: "Restart Indexer", command: "terrain.restartIndexer" },
            { label: "Show Log", command: "terrain.showLog" },
          ],
        },
        "*",
      ),
    );
    await page.waitForTimeout(300);
    await page.screenshot({ path: join(out, `error-${theme}.png`) });
    const focused = await page.evaluate(() => document.activeElement?.textContent);
    if (focused !== "Restart Indexer") fail(`${theme}: error screen does not focus its first action (focused: ${focused})`);

    if (errors.length) fail(`${theme}: page errors: ${errors.join(" | ")}`);
    writeFileSync(join(out, `audit-${theme}.json`), JSON.stringify(audit, null, 2));
    await page.close();
  }
} finally {
  await browser.close();
  server.kill();
}
console.log(failures.length ? `\n${failures.length} visual check(s) failed` : "\nall visual checks pass in three themes");
process.exit(failures.length ? 1 : 0);

/** Share of pixels that differ from `color` (or, with `near`, that are close to it). */
async function pixels(page, png, color, tolerance, near = false) {
  return page.evaluate(
    async ({ b64, color, tolerance, near }) => {
      const img = new Image();
      img.src = `data:image/png;base64,${b64}`;
      await img.decode();
      const c = document.createElement("canvas");
      c.width = img.width;
      c.height = img.height;
      const ctx = c.getContext("2d");
      ctx.drawImage(img, 0, 0);
      const d = ctx.getImageData(0, 0, c.width, c.height).data;
      let n = 0;
      for (let i = 0; i < d.length; i += 4) {
        const dist = Math.abs(d[i] - color[0]) + Math.abs(d[i + 1] - color[1]) + Math.abs(d[i + 2] - color[2]);
        if (near ? dist <= tolerance : dist > tolerance) n++;
      }
      return n / (d.length / 4);
    },
    { b64: png.toString("base64"), color, tolerance, near },
  );
}
