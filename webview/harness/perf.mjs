// Pan/zoom performance: drives wheel and drag input for a few seconds through the harness and
// reports frame intervals. Budget: 60 fps, no frame > 32 ms.
// Usage: node webview/harness/perf.mjs [url] [--gpu]
import { chromium } from "playwright";

const url = process.argv[2] ?? "http://localhost:4710/?theme=dark";
const gpu = process.argv.includes("--gpu");
const browser = await chromium.launch({
  headless: true,
  args: gpu ? ["--use-angle=d3d11", "--ignore-gpu-blocklist", "--enable-gpu-rasterization"] : ["--use-angle=swiftshader", "--enable-unsafe-swiftshader"],
});
const page = await browser.newPage({ viewport: { width: 1600, height: 1000 } });
await page.goto(url);
await page.evaluate(() => sessionStorage.clear());
await page.goto(url);
await page.waitForFunction(async () => (await window.__terrain.rpc("index/status", null)).result?.phase === "idle", null, { timeout: 600000, polling: 500 });
await page.waitForTimeout(1500);
const renderer = await page.evaluate(() => {
  const c = document.querySelector("canvas");
  const gl = c.getContext("webgl2");
  const ext = gl?.getExtension("WEBGL_debug_renderer_info");
  return ext ? gl.getParameter(ext.UNMASKED_RENDERER_WEBGL) : "canvas2d";
});
await page.evaluate(() => {
  window.__terrainApp.frames.length = 0;
  window.__longTasks = [];
  new PerformanceObserver((l) => {
    for (const e of l.getEntries()) window.__longTasks.push(Math.round(e.duration));
  }).observe({ type: "longtask" });
});
const elapsed = 5000;
// --cached: run the motion once to load tiles, then measure a second identical pass
// (separates rendering cost from tile streaming cost).
if (process.argv.includes("--cached")) {
  await page.evaluate((ms) => window.__terrainApp.motionTest(ms), elapsed);
  await page.waitForTimeout(1500);
  await page.evaluate(() => {
    window.__terrainApp.frames.length = 0;
    window.__longTasks.length = 0;
  });
}
const intervals = await page.evaluate((ms) => window.__terrainApp.motionTest(ms), elapsed);
const { frames, items, longTasks, breakdown } = await page.evaluate(() => ({ frames: window.__terrainApp.frames, items: window.__terrainApp.itemCount, longTasks: window.__longTasks, breakdown: window.__terrainApp.breakdown.slice(-300) }));
await browser.close();
const cpu = frames.map((f) => f[1]).sort((a, b) => a - b);
const sorted = [...intervals].sort((a, b) => a - b);
const p = (xs, q) => xs[Math.min(xs.length - 1, Math.floor(q * xs.length))];
const result = {
  renderer,
  frames: intervals.length,
  seconds: elapsed / 1000,
  fps: intervals.length / (elapsed / 1000),
  intervalP50: p(sorted, 0.5),
  intervalP99: p(sorted, 0.99),
  intervalMax: sorted[sorted.length - 1],
  framesOver32ms: intervals.filter((x) => x > 32).length,
  cpuP50: p(cpu, 0.5),
  cpuP99: p(cpu, 0.99),
  cpuMax: cpu[cpu.length - 1],
  itemsAtEnd: items,
  longTasksMs: longTasks,
  p90: ["assemble", "labels", "render"].map((n, i) => `${n} ${p(breakdown.map((b) => b[i]).sort((a, b) => a - b), 0.9).toFixed(1)}`).join(", "),
};
console.log(JSON.stringify(result, null, 2));
