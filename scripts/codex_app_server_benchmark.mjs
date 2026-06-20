#!/usr/bin/env node

import { spawn } from "node:child_process";
import { createHash } from "node:crypto";
import { once, setMaxListeners } from "node:events";
import { setTimeout as sleep } from "node:timers/promises";

const args = new Map();
for (let i = 2; i < process.argv.length; i += 1) {
  const arg = process.argv[i];
  if (!arg.startsWith("--")) continue;
  const next = process.argv[i + 1];
  if (next && !next.startsWith("--")) {
    args.set(arg, next);
    i += 1;
  } else {
    args.set(arg, "true");
  }
}

const iterations = Number(args.get("--iterations") ?? "50");
const port = Number(args.get("--port") ?? "18765");
const runWebSocket = args.get("--skip-websocket") !== "true";

function percentile(values, pct) {
  if (values.length === 0) return null;
  const sorted = [...values].sort((a, b) => a - b);
  const idx = Math.min(sorted.length - 1, Math.ceil((pct / 100) * sorted.length) - 1);
  return Number(sorted[idx].toFixed(3));
}

function request(id, method, params = {}) {
  return JSON.stringify({ jsonrpc: "2.0", id, method, params });
}

async function readLine(stream, timeoutMs) {
  let buffer = "";
  const deadline = performance.now() + timeoutMs;
  for (;;) {
    const chunk = stream.read();
    if (chunk) {
      buffer += chunk.toString("utf8");
      const newline = buffer.indexOf("\n");
      if (newline >= 0) return buffer.slice(0, newline);
    }
    if (performance.now() > deadline) throw new Error("timed out waiting for line");
    await once(stream, "readable");
  }
}

async function runStdioProbe() {
  const child = spawn("codex", ["app-server", "--stdio"], {
    stdio: ["pipe", "pipe", "pipe"],
  });
  child.stdout.setEncoding("utf8");
  child.stderr.setEncoding("utf8");
  const startedAt = performance.now();
  child.stdin.write(
    `${request(1, "initialize", {
      clientInfo: { name: "opensymphony-codex-benchmark", version: "0.0.0" },
      capabilities: {},
    })}\n`,
  );
  const line = await readLine(child.stdout, 5000);
  const latencyMs = performance.now() - startedAt;
  child.kill("SIGTERM");
  await Promise.race([once(child, "exit"), sleep(1000)]);
  return {
    transport: "stdio",
    initializeLatencyMs: Number(latencyMs.toFixed(3)),
    response: JSON.parse(line),
  };
}

async function waitForReadyz(url, timeoutMs = 5000) {
  const deadline = performance.now() + timeoutMs;
  let lastError = null;
  while (performance.now() < deadline) {
    try {
      const response = await fetch(url);
      if (response.ok) return true;
      lastError = new Error(`readyz returned ${response.status}`);
    } catch (error) {
      lastError = error;
    }
    await sleep(100);
  }
  throw lastError ?? new Error("readyz timed out");
}

async function openSocket(url) {
  const ws = new WebSocket(url);
  setMaxListeners(0, ws);
  await new Promise((resolve, reject) => {
    ws.addEventListener("open", resolve, { once: true });
    ws.addEventListener("error", reject, { once: true });
  });
  return ws;
}

function requestOverSocket(ws, id, method, params = {}) {
  const startedAt = performance.now();
  return new Promise((resolve, reject) => {
    const onMessage = (event) => {
      const parsed = JSON.parse(event.data);
      if (parsed.id !== id) return;
      ws.removeEventListener("message", onMessage);
      resolve({ latencyMs: performance.now() - startedAt, response: parsed });
    };
    ws.addEventListener("message", onMessage);
    ws.addEventListener("error", reject, { once: true });
    ws.send(request(id, method, params));
  });
}

async function runWebSocketProbe() {
  const child = spawn("codex", ["app-server", "--listen", `ws://127.0.0.1:${port}`], {
    stdio: ["ignore", "pipe", "pipe"],
  });
  let stderr = "";
  child.stderr.on("data", (chunk) => {
    stderr += chunk.toString("utf8");
  });

  await waitForReadyz(`http://127.0.0.1:${port}/readyz`);

  const ws = await openSocket(`ws://127.0.0.1:${port}`);
  const initialize = await requestOverSocket(ws, 1, "initialize", {
    clientInfo: { name: "opensymphony-codex-benchmark", version: "0.0.0" },
    capabilities: {},
  });

  const batchStartedAt = performance.now();
  const requests = [];
  for (let i = 0; i < iterations; i += 1) {
    requests.push(requestOverSocket(ws, i + 2, "thread/loaded/list", { limit: 1 }));
  }
  const responses = await Promise.all(requests);
  const elapsedMs = performance.now() - batchStartedAt;
  const latencies = responses.map((response) => response.latencyMs);

  ws.close();
  await sleep(100);
  const reconnectStartedAt = performance.now();
  const ws2 = await openSocket(`ws://127.0.0.1:${port}`);
  const reconnectInitialize = await requestOverSocket(ws2, iterations + 2, "initialize", {
    clientInfo: { name: "opensymphony-codex-benchmark-reconnect", version: "0.0.0" },
    capabilities: {},
  });
  const reconnectMs = performance.now() - reconnectStartedAt;
  ws2.close();

  child.kill("SIGTERM");
  await Promise.race([once(child, "exit"), sleep(1000)]);

  return {
    transport: "websocket_loopback",
    port,
    initializeLatencyMs: Number(initialize.latencyMs.toFixed(3)),
    queuedRequests: iterations,
    queuedResponses: responses.length,
    queueElapsedMs: Number(elapsedMs.toFixed(3)),
    requestsPerSecond: Number(((responses.length / elapsedMs) * 1000).toFixed(2)),
    latencyMs: {
      p50: percentile(latencies, 50),
      p95: percentile(latencies, 95),
      max: Number(Math.max(...latencies).toFixed(3)),
    },
    reconnectLatencyMs: Number(reconnectMs.toFixed(3)),
    reconnectResponse: reconnectInitialize.response.result ? "ok" : "missing_result",
    exposure: {
      listener: `ws://127.0.0.1:${port}`,
      localhostOnly: /binds localhost only/.test(stderr),
      authModesFromHelp: ["capability-token", "signed-bearer-token"],
    },
  };
}

async function runHelpProbe() {
  const child = spawn("codex", ["app-server", "--help"], {
    stdio: ["ignore", "pipe", "pipe"],
  });
  const chunks = [];
  child.stdout.on("data", (chunk) => chunks.push(chunk));
  await once(child, "exit");
  const help = Buffer.concat(chunks).toString("utf8");
  return {
    transport: "websocket_secure_exposure",
    helpSha256: createHash("sha256").update(help).digest("hex"),
    hasCapabilityTokenMode: help.includes("capability-token"),
    hasSignedBearerMode: help.includes("signed-bearer-token"),
    hasTokenFileFlag: help.includes("--ws-token-file"),
    hasSharedSecretFlag: help.includes("--ws-shared-secret-file"),
  };
}

const report = {
  generatedAt: new Date().toISOString(),
  codexVersion: null,
  stdio: null,
  websocket: null,
  secureExposure: null,
};

try {
  const version = spawn("codex", ["--version"], { stdio: ["ignore", "pipe", "pipe"] });
  const chunks = [];
  version.stdout.on("data", (chunk) => chunks.push(chunk));
  await once(version, "exit");
  report.codexVersion = Buffer.concat(chunks).toString("utf8").trim();
  report.stdio = await runStdioProbe();
  report.secureExposure = await runHelpProbe();
  if (runWebSocket) {
    report.websocket = await runWebSocketProbe();
  }
  console.log(JSON.stringify(report, null, 2));
} catch (error) {
  console.error(error.stack || String(error));
  process.exit(1);
}
