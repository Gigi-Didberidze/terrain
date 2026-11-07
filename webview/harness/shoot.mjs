// Screenshots the map through the harness. Usage:
//   node webview/harness/shoot.mjs <url-base> <outDir> [theme ...]
// Waits until indexing is idle and tiles are drawn, then captures the full view.
import { chromium } from "playwright";
import { mkdirSync } from "node:fs";
import { join } from "node:path";

const args = process.argv.slice(2);
const selIdx = args.indexOf("--select");
// --select <search query>: select the first hit (shows its edges) before the dark screenshot.
const select = selIdx >= 0 ? args.splice(selIdx, 2)[1] : null;
const [base = "http://localhost:4710", out = "shots", ...themes] = args;
mkdirSync(out, { recursive: true });
const browser = await chromium.launch({ args: ["--use-angle=swiftshader", "--enable-unsafe-swiftshader"] });
const page = await browser.newPage({ viewport: { width: 1280, height: 800 }, deviceScaleFactor: 1 });
const logs = [];
page.on("console", (m) => logs.push(`${m.type()}: ${m.text()}`));
page.on("pageerror", (e) => logs.push(`pageerror: ${e.message}`));
for (const theme of themes.length ? themes : ["dark", "light", "hc"]) {
  await page.goto(`${base}/?theme=${theme}`);
  await page.evaluate(() => sessionStorage.clear());
  await page.goto(`${base}/?theme=${theme}`);
  // Wait for the indexer to be idle, then for tiles to settle.
  await page.waitForFunction(async () => {
    const r = await window.__terrain.rpc("index/status", null);
    return r.result && r.result.phase === "idle";
  }, null, { timeout: 120000, polling: 250 });
  await page.waitForTimeout(800);
  await page.screenshot({ path: join(out, `map-${theme}.png`) });
  if (theme === "dark") {
    // Load-bearing lens, then back to Structure.
    await page.keyboard.press("2");
    await page.waitForTimeout(1200);
    await page.screenshot({ path: join(out, `map-${theme}-load.png`) });
    // History lenses: churn and ownership (coupling needs a selection, below).
    for (const [key, name] of [["3", "churn"], ["5", "owner"]]) {
      await page.keyboard.press(key);
      await page.waitForTimeout(1200);
      await page.screenshot({ path: join(out, `map-${theme}-${name}.png`) });
    }
    await page.keyboard.press("1");
    await page.waitForTimeout(600);
  }
  if (theme === "dark" && select) {
    await page.evaluate(async (q) => {
      const r = await window.__terrain.rpc("search", { query: q, limit: 1 });
      const hit = r.result.hits[0];
      if (hit) await window.__terrainApp.reveal(hit.slot, true);
    }, select);
    await page.waitForTimeout(1500);
    await page.screenshot({ path: join(out, `map-${theme}-selected.png`) });
    await page.keyboard.press("4");
    await page.waitForTimeout(1500);
    await page.screenshot({ path: join(out, `map-${theme}-coupling.png`) });
    await page.keyboard.press("1");
    await page.waitForTimeout(600);
  }
  if (theme === "dark") {
    // Zoom into the upper-left quadrant to check semantic zoom and labels.
    await page.mouse.move(330, 260);
    for (let i = 0; i < 12; i++) await page.mouse.wheel(0, -120);
    await page.waitForTimeout(1200);
    await page.screenshot({ path: join(out, `map-${theme}-zoom.png`) });
  }
}
await browser.close();
if (logs.length) console.log(logs.join("\n"));
