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
await page.waitForFunction(async () => (await window.__compass.rpc("index/status", null)).result?.phase === "idle", null, { timeout: 600000, polling: 500 });
await page.waitForTimeout(1500);
const renderer = await page.evaluate(() => {
  const c = document.querySelector("canvas");
  const gl = c.getContext("webgl2");
  const ext = gl?.getExtension("WEBGL_debug_renderer_info");
  return ext ? gl.getParameter(ext.UNMASKED_RENDERER_WEBGL) : "canvas2d";
});
await page.evaluate(() => (window.__compassApp.frames.length = 0));
const elapsed = 5000;
const intervals = await page.evaluate((ms) => window.__compassApp.motionTest(ms), elapsed);
const { frames, items } = await page.evaluate(() => ({ frames: window.__compassApp.frames, items: window.__compassApp.itemCount }));
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
};
console.log(JSON.stringify(result, null, 2));
