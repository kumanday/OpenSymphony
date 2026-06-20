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
const requestTimeoutMs = Number(args.get("--request-timeout-ms") ?? "5000");

function percentile(values, pct) {
  if (values.length === 0) return null;
  const sorted = [...values].sort((a, b) => a - b);
  const idx = Math.min(sorted.length - 1, Math.ceil((pct / 100) * sorted.length) - 1);
  return Number(sorted[idx].toFixed(3));
}

function request(id, method, params = {}) {
  return JSON.stringify({ jsonrpc: "2.0", id, method, params });
}

async function terminateChild(child, graceMs = 1000) {
  if (child.exitCode !== null || child.signalCode !== null) return;
  const exited = once(child, "exit").then(() => true);
  child.kill("SIGTERM");
  if (await Promise.race([exited, sleep(graceMs).then(() => false)])) return;
  child.kill("SIGKILL");
  await Promise.race([exited, sleep(graceMs)]);
}

function assertJsonRpcResult(label, response) {
  if (response == null || typeof response !== "object") {
    throw new Error(`${label} returned non-object JSON-RPC response: ${JSON.stringify(response)}`);
  }
  if (response.error) {
    throw new Error(`${label} returned JSON-RPC error: ${JSON.stringify(response.error)}`);
  }
  if (!Object.hasOwn(response, "result")) {
    throw new Error(`${label} did not include a JSON-RPC result`);
  }
}

function waitForStreamProgress(stream, timeoutMs) {
  return new Promise((resolve, reject) => {
    const cleanup = () => {
      clearTimeout(timeout);
      stream.removeListener("readable", onReadable);
      stream.removeListener("end", onEnd);
      stream.removeListener("close", onClose);
      stream.removeListener("error", onError);
    };
    const complete = (event) => {
      cleanup();
      resolve(event);
    };
    const onReadable = () => complete("readable");
    const onEnd = () => complete("end");
    const onClose = () => complete("close");
    const onError = (error) => {
      cleanup();
      reject(error);
    };
    const timeout = setTimeout(() => complete("timeout"), timeoutMs);
    stream.once("readable", onReadable);
    stream.once("end", onEnd);
    stream.once("close", onClose);
    stream.once("error", onError);
  });
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
    const remainingMs = Math.max(1, deadline - performance.now());
    const event = await waitForStreamProgress(stream, remainingMs);
    if (event === "timeout") throw new Error("timed out waiting for line");
    if (event === "end" || event === "close") {
      throw new Error(`stream ${event} before a complete line`);
    }
  }
}

async function collectChildOutput(child, label, timeoutMs = requestTimeoutMs) {
  const stdout = [];
  const stderr = [];
  child.stdout?.on("data", (chunk) => stdout.push(chunk));
  child.stderr?.on("data", (chunk) => stderr.push(chunk));

  const result = await Promise.race([
    once(child, "exit").then(([code, signal]) => ({ code, signal, timedOut: false })),
    sleep(timeoutMs).then(() => ({ code: null, signal: null, timedOut: true })),
  ]);

  if (result.timedOut) {
    await terminateChild(child);
    throw new Error(`${label} timed out after ${timeoutMs}ms`);
  }

  if (result.code !== 0) {
    const suffix = result.signal ? ` signal ${result.signal}` : ` code ${result.code}`;
    const stderrText = Buffer.concat(stderr).toString("utf8").trim();
    throw new Error(`${label} exited with${suffix}${stderrText ? `: ${stderrText}` : ""}`);
  }

  return Buffer.concat(stdout).toString("utf8");
}

async function runStdioProbe() {
  const child = spawn("codex", ["app-server", "--stdio"], {
    stdio: ["pipe", "pipe", "pipe"],
  });
  try {
    child.stdout.setEncoding("utf8");
    child.stderr.setEncoding("utf8");
    const startedAt = performance.now();
    child.stdin.write(
      `${request(1, "initialize", {
        clientInfo: { name: "opensymphony-codex-benchmark", version: "0.0.0" },
        capabilities: {},
      })}\n`,
    );
    const line = await readLine(child.stdout, requestTimeoutMs);
    const latencyMs = performance.now() - startedAt;
    const response = JSON.parse(line);
    assertJsonRpcResult("stdio initialize", response);
    return {
      transport: "stdio",
      initializeLatencyMs: Number(latencyMs.toFixed(3)),
      response,
    };
  } finally {
    await terminateChild(child);
  }
}

async function waitForReadyz(url, timeoutMs = 5000) {
  const deadline = performance.now() + timeoutMs;
  let lastError = null;
  while (performance.now() < deadline) {
    const controller = new AbortController();
    const remainingMs = Math.max(1, deadline - performance.now());
    const abort = setTimeout(() => controller.abort(), Math.min(remainingMs, 500));
    try {
      const response = await fetch(url, { signal: controller.signal });
      if (response.ok) return true;
      lastError = new Error(`readyz returned ${response.status}`);
    } catch (error) {
      lastError = error;
    } finally {
      clearTimeout(abort);
    }
    await sleep(100);
  }
  throw lastError ?? new Error("readyz timed out");
}

async function openSocket(url, timeoutMs = requestTimeoutMs) {
  const ws = new WebSocket(url);
  setMaxListeners(0, ws);
  await new Promise((resolve, reject) => {
    const cleanup = () => {
      clearTimeout(timeout);
      ws.removeEventListener("open", onOpen);
      ws.removeEventListener("error", onError);
    };
    const onOpen = () => {
      cleanup();
      resolve();
    };
    const onError = (error) => {
      cleanup();
      reject(error);
    };
    const timeout = setTimeout(() => {
      cleanup();
      ws.close();
      reject(new Error(`WebSocket connection to ${url} timed out after ${timeoutMs}ms`));
    }, timeoutMs);
    ws.addEventListener("open", onOpen);
    ws.addEventListener("error", onError);
  });
  return ws;
}

function requestOverSocket(ws, id, method, params = {}, timeoutMs = requestTimeoutMs) {
  const startedAt = performance.now();
  return new Promise((resolve, reject) => {
    const cleanup = () => {
      clearTimeout(timeout);
      ws.removeEventListener("message", onMessage);
      ws.removeEventListener("error", onError);
    };
    const onMessage = (event) => {
      let parsed;
      try {
        parsed = JSON.parse(event.data);
      } catch (error) {
        cleanup();
        reject(error);
        return;
      }
      if (parsed.id !== id) return;
      cleanup();
      resolve({ latencyMs: performance.now() - startedAt, response: parsed });
    };
    const onError = (error) => {
      cleanup();
      reject(error);
    };
    const timeout = setTimeout(() => {
      cleanup();
      reject(new Error(`${method} request ${id} timed out after ${timeoutMs}ms`));
    }, timeoutMs);
    ws.addEventListener("message", onMessage);
    ws.addEventListener("error", onError);
    ws.send(request(id, method, params));
  });
}

async function runWebSocketProbe(secureExposure) {
  const child = spawn("codex", ["app-server", "--listen", `ws://127.0.0.1:${port}`], {
    stdio: ["ignore", "pipe", "pipe"],
  });
  let stderr = "";
  child.stderr.on("data", (chunk) => {
    stderr += chunk.toString("utf8");
  });

  let ws = null;
  let ws2 = null;
  try {
    await waitForReadyz(`http://127.0.0.1:${port}/readyz`);

    ws = await openSocket(`ws://127.0.0.1:${port}`);
    const initialize = await requestOverSocket(ws, 1, "initialize", {
      clientInfo: { name: "opensymphony-codex-benchmark", version: "0.0.0" },
      capabilities: {},
    });
    assertJsonRpcResult("websocket initialize", initialize.response);

    const batchStartedAt = performance.now();
    const requests = [];
    for (let i = 0; i < iterations; i += 1) {
      requests.push(requestOverSocket(ws, i + 2, "thread/loaded/list", { limit: 1 }));
    }
    const responses = await Promise.all(requests);
    for (const response of responses) {
      assertJsonRpcResult("websocket queued request", response.response);
    }
    const elapsedMs = performance.now() - batchStartedAt;
    const latencies = responses.map((response) => response.latencyMs);

    ws.close();
    await sleep(100);
    const reconnectStartedAt = performance.now();
    ws2 = await openSocket(`ws://127.0.0.1:${port}`);
    const reconnectInitialize = await requestOverSocket(ws2, iterations + 2, "initialize", {
      clientInfo: { name: "opensymphony-codex-benchmark-reconnect", version: "0.0.0" },
      capabilities: {},
    });
    assertJsonRpcResult("websocket reconnect initialize", reconnectInitialize.response);
    const reconnectMs = performance.now() - reconnectStartedAt;

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
      reconnectResponse: "ok",
      exposure: {
        listener: `ws://127.0.0.1:${port}`,
        localhostOnly: /binds localhost only/.test(stderr),
        authModesFromHelp: [
          ...(secureExposure.hasCapabilityTokenMode ? ["capability-token"] : []),
          ...(secureExposure.hasSignedBearerMode ? ["signed-bearer-token"] : []),
        ],
      },
    };
  } finally {
    if (ws && ws.readyState < WebSocket.CLOSING) ws.close();
    if (ws2 && ws2.readyState < WebSocket.CLOSING) ws2.close();
    await terminateChild(child);
  }
}

async function runHelpProbe() {
  const child = spawn("codex", ["app-server", "--help"], {
    stdio: ["ignore", "pipe", "pipe"],
  });
  const help = await collectChildOutput(child, "codex app-server --help");
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
  report.codexVersion = (await collectChildOutput(version, "codex --version")).trim();
  report.stdio = await runStdioProbe();
  report.secureExposure = await runHelpProbe();
  if (runWebSocket) {
    report.websocket = await runWebSocketProbe(report.secureExposure);
  }
  console.log(JSON.stringify(report, null, 2));
} catch (error) {
  console.error(error.stack || String(error));
  process.exit(1);
}
