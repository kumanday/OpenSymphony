/**
 * Browser app entrypoint for Vite.
 *
 * This file is imported by index.html and serves as the root module
 * for the browser bundle. It imports only shared frontend packages
 * and never references Tauri or desktop-only APIs.
 */

import { createWebAppConfig } from "./config.js";

const config = createWebAppConfig();

// Mount a minimal root placeholder until the UI framework is added.
const root = document.getElementById("root");
if (root) {
  root.innerHTML = `
    <style>
      body { font-family: system-ui, sans-serif; margin: 0; padding: 2rem; background: #0d1117; color: #c9d1d9; }
      h1 { font-size: 1.4rem; margin: 0 0 0.5rem; }
      .badge { display: inline-block; padding: 0.2em 0.6em; border-radius: 4px; background: #21262d; font-size: 0.85rem; }
      .status { margin-top: 1rem; }
      .status-ok { color: #3fb950; }
      .status-warn { color: #d29922; }
    </style>
    <h1>OpenSymphony Web Client</h1>
    <div><span class="badge">gateway: ${config.gatewayUrl}</span></div>
    <div><span class="badge">mode: ${config.gatewayServed ? "gateway-served" : "separate"}</span></div>
    <div class="status status-ok">Browser shell ready.</div>
  `;
}

export { config as webConfig };
