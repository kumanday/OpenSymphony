/**
 * Browser app entrypoint for Vite.
 *
 * This file is imported by index.html and serves as the root module
 * for the browser bundle. It imports only shared frontend packages
 * and never references Tauri or desktop-only APIs.
 */

import { createWebAppConfig } from "./config.js";
import { createTerminalRenderer } from "@opensymphony/ui-core";
import { createTerminalViewer } from "./terminal-viewer.js";
import {
  generateBurstFrames,
  generateRealisticSession,
  generateBurstySession,
  runAllBenchmarks,
  formatBenchmarkReport,
} from "@opensymphony/ui-core";

const config = createWebAppConfig();

// Mount a minimal root placeholder until the UI framework is added.
const root = document.getElementById("root");
if (root) {
  root.innerHTML = `
    <style>
      body { font-family: system-ui, sans-serif; margin: 0; padding: 2rem; background: #0d1117; color: #c9d1d9; }
      h1 { font-size: 1.4rem; margin: 0 0 0.5rem; }
      h2 { font-size: 1.1rem; margin: 1rem 0 0.5rem; color: #58a6ff; }
      .badge { display: inline-block; padding: 0.2em 0.6em; border-radius: 4px; background: #21262d; font-size: 0.85rem; margin: 0.2em; }
      .status { margin-top: 1rem; }
      .status-ok { color: #3fb950; }
      .status-warn { color: #d29922; }
      .controls { margin: 1rem 0; display: flex; gap: 0.5rem; flex-wrap: wrap; }
      .btn {
        padding: 0.5rem 1rem;
        background: #21262d;
        border: 1px solid #30363d;
        border-radius: 6px;
        color: #c9d1d9;
        cursor: pointer;
        font-size: 0.9rem;
        transition: background 0.15s;
      }
      .btn:hover { background: #30363d; }
      .btn-primary { background: #238636; border-color: #2ea043; }
      .btn-primary:hover { background: #2ea043; }
      #terminal-container { margin: 1rem 0; }
      #benchmark-output {
        background: #161b22;
        border: 1px solid #30363d;
        border-radius: 6px;
        padding: 1rem;
        font-family: Menlo, Monaco, 'Courier New', monospace;
        font-size: 0.85rem;
        max-height: 400px;
        overflow-y: auto;
        white-space: pre-wrap;
        color: #8b949e;
      }
    </style>
    <h1>OpenSymphony Web Client</h1>
    <div><span class="badge">gateway: ${config.gatewayUrl}</span></div>
    <div><span class="badge">mode: ${config.gatewayServed ? "gateway-served" : "separate"}</span></div>
    <div class="status status-ok">Browser shell ready.</div>
    
    <h2>Terminal Renderer Demo</h2>
    <div id="terminal-container"></div>
    <div class="controls">
      <button class="btn btn-primary" id="btn-burst-100">Burst 100 frames</button>
      <button class="btn btn-primary" id="btn-burst-1000">Burst 1000 frames</button>
      <button class="btn btn-primary" id="btn-session-realistic">Realistic session</button>
      <button class="btn btn-primary" id="btn-session-bursty">Bursty session</button>
      <button class="btn" id="btn-benchmark">Run benchmarks</button>
      <button class="btn" id="btn-clear">Clear terminal</button>
    </div>
    <div id="benchmark-output">Benchmark results will appear here...</div>
  `;

  // Initialize terminal renderer
  const renderer = createTerminalRenderer({ maxBufferCapacity: 2000 });
  const container = document.getElementById("terminal-container")!;
  const viewer = createTerminalViewer(renderer, { container });

  // Wire up buttons
  document.getElementById("btn-burst-100")?.addEventListener("click", () => {
    const frames = generateBurstFrames(100, { includeAnsiCodes: true });
    for (const frame of frames) {
      renderer.queueFrame(frame.content, frame.encoding, frame);
    }
  });

  document.getElementById("btn-burst-1000")?.addEventListener("click", () => {
    const frames = generateBurstFrames(1000, { includeAnsiCodes: true });
    for (const frame of frames) {
      renderer.queueFrame(frame.content, frame.encoding, frame);
    }
  });

  document.getElementById("btn-session-realistic")?.addEventListener("click", () => {
    const output = document.getElementById("benchmark-output")!;
    output.textContent = "Generating realistic session (30s @ 30fps)...";
    
    const frames = generateRealisticSession(30000, 30);
    for (const frame of frames) {
      renderer.queueFrame(frame.content, frame.encoding, frame);
    }
    
    output.textContent = `Loaded ${frames.length} frames. Check terminal above.`;
  });

  document.getElementById("btn-session-bursty")?.addEventListener("click", () => {
    const output = document.getElementById("benchmark-output")!;
    output.textContent = "Generating bursty session...";
    
    const frames = generateBurstySession(30000, 2000, 100, 5);
    for (const frame of frames) {
      renderer.queueFrame(frame.content, frame.encoding, frame);
    }
    
    output.textContent = `Loaded ${frames.length} frames. Check terminal above.`;
  });

  document.getElementById("btn-benchmark")?.addEventListener("click", async () => {
    const output = document.getElementById("benchmark-output")!;
    output.textContent = "Running benchmarks... This may take a moment.\n";

    try {
      const results = await runAllBenchmarks({ warmupFrames: 50, testFrames: 500 });

      let report = "";
      for (const result of results) {
        report += formatBenchmarkReport(result) + "\n\n";
      }

      output.textContent = report;
    } catch (err) {
      output.textContent = `Benchmark error: ${err instanceof Error ? err.message : String(err)}`;
    }
  });

  document.getElementById("btn-clear")?.addEventListener("click", () => {
    renderer.clear();
    const output = document.getElementById("benchmark-output")!;
    output.textContent = "Terminal cleared.";
  });

  // Expose for debugging
  (window as any).terminalRenderer = renderer;
  (window as any).terminalViewer = viewer;
}

export { config as webConfig };
