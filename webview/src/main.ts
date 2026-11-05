// Webview entry point. Plain TypeScript, no framework: the UI is small and must render instantly.
import "./main.css";
import { App } from "./app.ts";

const app = new App(document.getElementById("app")!);
// Exposed for Playwright performance and visual tests.
(window as unknown as { __compassApp: App }).__compassApp = app;
