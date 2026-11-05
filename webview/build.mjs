import * as esbuild from "esbuild";

const watch = process.argv.includes("--watch");
const options = {
  entryPoints: ["src/main.ts"],
  bundle: true,
  outdir: "../extension/dist/webview",
  format: "esm",
  splitting: true,
  platform: "browser",
  target: "es2022",
  sourcemap: true,
  minify: !watch,
  logLevel: "info",
};
if (watch) await (await esbuild.context(options)).watch();
else await esbuild.build(options);
