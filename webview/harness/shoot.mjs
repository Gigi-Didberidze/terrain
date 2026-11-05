// Screenshots the map through the harness. Usage:
//   node webview/harness/shoot.mjs <url-base> <outDir> [theme ...]
// Waits until indexing is idle and tiles are drawn, then captures the full view.
import { chromium } from "playwright";
import { mkdirSync } from "node:fs";
import { join } from "node:path";

const [base = "http://localhost:4710", out = "shots", ...themes] = process.argv.slice(2);
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
    const r = await window.__compass.rpc("index/status", null);
    return r.result && r.result.phase === "idle";
  }, null, { timeout: 120000, polling: 250 });
  await page.waitForTimeout(800);
  await page.screenshot({ path: join(out, `map-${theme}.png`) });
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
