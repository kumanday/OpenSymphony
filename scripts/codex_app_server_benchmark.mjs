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

function parseIntegerOption(flag, defaultValue, min, max) {
  const raw = args.get(flag) ?? String(defaultValue);
  if (!/^\d+$/.test(raw)) {
    console.error(`${flag} must be a base-10 integer from ${min} to ${max}; received ${JSON.stringify(raw)}`);
    process.exit(1);
  }
  const value = Number.parseInt(raw, 10);
  if (!Number.isInteger(value) || value < min || value > max) {
    console.error(`${flag} must be a base-10 integer from ${min} to ${max}; received ${JSON.stringify(raw)}`);
    process.exit(1);
  }
  return value;
}

const iterations = parseIntegerOption("--iterations", 50, 1, 100000);
const port = parseIntegerOption("--port", 18765, 1, 65535);
const runWebSocket = args.get("--skip-websocket") !== "true";
const requestTimeoutMs = parseIntegerOption("--request-timeout-ms", 5000, 1, 300000);
const activeChildren = new Set();
const activeSockets = new Set();

function assertWebSocketRuntime() {
  const nodeVersion = process.versions?.node ?? "unknown";
  if (typeof globalThis.WebSocket !== "function" || typeof globalThis.fetch !== "function") {
    throw new Error(
      `WebSocket benchmark requires Node.js 22+ globals WebSocket and fetch; current Node.js is ${nodeVersion}. Use --skip-websocket to run stdio-only probes.`,
    );
  }
}

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

function trackChild(child) {
  activeChildren.add(child);
  child.once("exit", () => activeChildren.delete(child));
  return child;
}

function trackSocket(ws) {
  activeSockets.add(ws);
  ws.addEventListener("close", () => activeSockets.delete(ws), { once: true });
  return ws;
}

async function cleanupActiveResources() {
  for (const ws of activeSockets) {
    if (ws.readyState < WebSocket.CLOSING) ws.close();
  }
  await Promise.all([...activeChildren].map((child) => terminateChild(child)));
}

for (const signal of ["SIGINT", "SIGTERM"]) {
  process.once(signal, () => {
    const exitCode = signal === "SIGINT" ? 130 : 143;
    cleanupActiveResources().finally(() => process.exit(exitCode));
  });
}

function assertJsonRpcResult(label, response) {
  if (response == null || typeof response !== "object") {
    throw new Error(`${label} returned non-object JSON-RPC response: ${JSON.stringify(response)}`);
  }
  if (response.error) {
    throw new Error(`${label} returned JSON-RPC error: ${JSON.stringify(response.error)}`);
  }
  if (Object.hasOwn(response, "jsonrpc") && response.jsonrpc !== "2.0") {
    throw new Error(`${label} returned unsupported JSON-RPC version: ${JSON.stringify(response.jsonrpc)}`);
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

class LineReader {
  constructor(stream) {
    this.stream = stream;
    this.buffer = "";
  }

  async readLine(timeoutMs) {
    const deadline = performance.now() + timeoutMs;
    for (;;) {
      const newline = this.buffer.indexOf("\n");
      if (newline >= 0) {
        const line = this.buffer.slice(0, newline);
        this.buffer = this.buffer.slice(newline + 1);
        return line;
      }

      const chunk = this.stream.read();
      if (chunk) {
        this.buffer += chunk.toString("utf8");
        continue;
      }

      if (performance.now() > deadline) throw new Error("timed out waiting for line");
      const remainingMs = Math.max(1, deadline - performance.now());
      const event = await waitForStreamProgress(this.stream, remainingMs);
      if (event === "timeout") throw new Error("timed out waiting for line");
      if (event === "end" || event === "close") {
        throw new Error(`stream ${event} before a complete line`);
      }
    }
  }
}

async function readJsonRpcResponse(reader, label, timeoutMs) {
  const deadline = performance.now() + timeoutMs;
  for (;;) {
    const remainingMs = Math.max(1, deadline - performance.now());
    if (performance.now() > deadline) {
      throw new Error(`${label} timed out waiting for JSON-RPC response`);
    }
    const line = await reader.readLine(remainingMs);
    let response;
    try {
      response = JSON.parse(line);
    } catch {
      continue;
    }
    if (
      response != null &&
      typeof response === "object" &&
      Object.hasOwn(response, "id") &&
      (Object.hasOwn(response, "result") || Object.hasOwn(response, "error"))
    ) {
      return response;
    }
  }
}

function writeToStream(stream, data, label, timeoutMs) {
  return new Promise((resolve, reject) => {
    const cleanup = () => {
      clearTimeout(timeout);
      stream.removeListener("error", onError);
    };
    const onError = (error) => {
      cleanup();
      reject(error);
    };
    const timeout = setTimeout(() => {
      cleanup();
      reject(new Error(`${label} write timed out after ${timeoutMs}ms`));
    }, timeoutMs);
    stream.once("error", onError);
    stream.write(data, (error) => {
      cleanup();
      if (error) reject(error);
      else resolve();
    });
  });
}

function endStream(stream, label, timeoutMs) {
  return new Promise((resolve, reject) => {
    const cleanup = () => {
      clearTimeout(timeout);
      stream.removeListener("error", onError);
    };
    const onError = (error) => {
      cleanup();
      reject(error);
    };
    const timeout = setTimeout(() => {
      cleanup();
      reject(new Error(`${label} end timed out after ${timeoutMs}ms`));
    }, timeoutMs);
    stream.once("error", onError);
    stream.end((error) => {
      cleanup();
      if (error) reject(error);
      else resolve();
    });
  });
}

async function collectChildOutput(child, label, timeoutMs = requestTimeoutMs) {
  const stdout = [];
  const stderr = [];
  child.stdout?.on("data", (chunk) => stdout.push(chunk));
  child.stderr?.on("data", (chunk) => stderr.push(chunk));

  const result = await Promise.race([
    once(child, "exit").then(([code, signal]) => ({ code, signal, timedOut: false })),
    once(child, "error").then(([error]) => ({ error, timedOut: false })),
    sleep(timeoutMs).then(() => ({ code: null, signal: null, timedOut: true })),
  ]);

  if (result.error) {
    throw new Error(`${label} failed to start: ${result.error.message}`);
  }

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
  const child = trackChild(
    spawn("codex", ["app-server", "--stdio"], {
      stdio: ["pipe", "pipe", "pipe"],
    }),
  );
  try {
    const childError = once(child, "error").then(([error]) => {
      throw new Error(`codex app-server --stdio failed to start: ${error.message}`);
    });
    childError.catch(() => {});
    child.stdout.setEncoding("utf8");
    child.stderr.setEncoding("utf8");
    let stderr = "";
    child.stderr.on("data", (chunk) => {
      stderr += chunk.toString("utf8");
    });
    const stdout = new LineReader(child.stdout);
    const startedAt = performance.now();
    await Promise.race([
      writeToStream(
        child.stdin,
        `${request(1, "initialize", {
          clientInfo: { name: "opensymphony-codex-benchmark", version: "0.0.0" },
          capabilities: {},
        })}\n`,
        "stdio initialize",
        requestTimeoutMs,
      ),
      childError,
    ]);
    const response = await Promise.race([
      readJsonRpcResponse(stdout, "stdio initialize", requestTimeoutMs),
      childError,
    ]);
    const latencyMs = performance.now() - startedAt;
    assertJsonRpcResult("stdio initialize", response);
    await Promise.race([
      endStream(child.stdin, "stdio initialize", requestTimeoutMs),
      childError,
    ]);
    return {
      transport: "stdio",
      initializeLatencyMs: Number(latencyMs.toFixed(3)),
      response,
      stderrBytes: Buffer.byteLength(stderr, "utf8"),
    };
  } finally {
    await terminateChild(child);
  }
}

async function waitForReadyz(url, timeoutMs = requestTimeoutMs) {
  const deadline = performance.now() + timeoutMs;
  let lastError = null;
  while (performance.now() < deadline) {
    const controller = new AbortController();
    const remainingMs = Math.max(1, deadline - performance.now());
    const abort = setTimeout(() => controller.abort(), Math.min(remainingMs, 500));
    try {
      const response = await fetch(url, { signal: controller.signal });
      await response.arrayBuffer();
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
  const ws = trackSocket(new WebSocket(url));
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

function waitForSocketClose(ws, timeoutMs = requestTimeoutMs) {
  if (ws.readyState === WebSocket.CLOSED) return Promise.resolve();
  return new Promise((resolve, reject) => {
    const cleanup = () => {
      clearTimeout(timeout);
      ws.removeEventListener("close", onClose);
    };
    const onClose = () => {
      cleanup();
      resolve();
    };
    const timeout = setTimeout(() => {
      cleanup();
      reject(new Error(`WebSocket close timed out after ${timeoutMs}ms`));
    }, timeoutMs);
    ws.addEventListener("close", onClose);
  });
}

class WebSocketJsonRpcClient {
  constructor(ws) {
    this.ws = ws;
    this.pending = new Map();
    this.onMessage = this.onMessage.bind(this);
    this.onError = this.onError.bind(this);
    this.onClose = this.onClose.bind(this);
    ws.addEventListener("message", this.onMessage);
    ws.addEventListener("error", this.onError);
    ws.addEventListener("close", this.onClose);
  }

  request(id, method, params = {}, timeoutMs = requestTimeoutMs) {
    const startedAt = performance.now();
    const key = String(id);
    if (this.pending.has(key)) {
      throw new Error(`duplicate JSON-RPC request id ${key}`);
    }
    return new Promise((resolve, reject) => {
      const timeout = setTimeout(() => {
        this.pending.delete(key);
        reject(new Error(`${method} request ${id} timed out after ${timeoutMs}ms`));
      }, timeoutMs);
      this.pending.set(key, {
        method,
        resolve: (response) => {
          clearTimeout(timeout);
          resolve({ latencyMs: performance.now() - startedAt, response });
        },
        reject: (error) => {
          clearTimeout(timeout);
          reject(error);
        },
      });
      try {
        this.ws.send(request(id, method, params));
      } catch (error) {
        this.pending.delete(key);
        clearTimeout(timeout);
        reject(error);
      }
    });
  }

  onMessage(event) {
    let parsed;
    try {
      parsed = JSON.parse(event.data);
    } catch (error) {
      this.rejectAll(error);
      return;
    }
    if (parsed == null || typeof parsed !== "object" || !Object.hasOwn(parsed, "id")) return;
    const pending = this.pending.get(String(parsed.id));
    if (!pending) return;
    this.pending.delete(String(parsed.id));
    pending.resolve(parsed);
  }

  onError(error) {
    this.rejectAll(error);
  }

  onClose() {
    this.rejectAll(new Error("WebSocket closed before all pending JSON-RPC responses arrived"));
  }

  rejectAll(error) {
    for (const [key, pending] of this.pending) {
      this.pending.delete(key);
      pending.reject(error);
    }
  }

  dispose() {
    this.ws.removeEventListener("message", this.onMessage);
    this.ws.removeEventListener("error", this.onError);
    this.ws.removeEventListener("close", this.onClose);
    this.rejectAll(new Error("WebSocket JSON-RPC client disposed"));
  }
}

async function runWebSocketProbe(secureExposure) {
  const child = trackChild(
    spawn("codex", ["app-server", "--listen", `ws://127.0.0.1:${port}`], {
      stdio: ["ignore", "pipe", "pipe"],
    }),
  );
  const stdoutChunks = [];
  const stderrChunks = [];
  const decodeOutput = () => ({
    stdout: Buffer.concat(stdoutChunks).toString("utf8"),
    stderr: Buffer.concat(stderrChunks).toString("utf8"),
  });
  child.stdout.on("data", (chunk) => {
    stdoutChunks.push(chunk);
  });
  child.stderr.on("data", (chunk) => {
    stderrChunks.push(chunk);
  });

  let ws = null;
  let ws2 = null;
  let client = null;
  let reconnectClient = null;
  try {
    const exitedBeforeReadyz = once(child, "exit").then(([code, signal]) => {
      const { stdout, stderr } = decodeOutput();
      const suffix = signal ? `signal ${signal}` : `code ${code}`;
      throw new Error(
        `codex app-server exited before readyz with ${suffix}; stdout=${JSON.stringify(stdout.trim())}; stderr=${JSON.stringify(stderr.trim())}`,
      );
    });
    exitedBeforeReadyz.catch(() => {});
    const failedBeforeReadyz = once(child, "error").then(([error]) => {
      throw new Error(`codex app-server failed to start before readyz: ${error.message}`);
    });
    failedBeforeReadyz.catch(() => {});
    await Promise.race([
      waitForReadyz(`http://127.0.0.1:${port}/readyz`, requestTimeoutMs),
      exitedBeforeReadyz,
      failedBeforeReadyz,
    ]);

    ws = await openSocket(`ws://127.0.0.1:${port}`);
    client = new WebSocketJsonRpcClient(ws);
    const initialize = await client.request(1, "initialize", {
      clientInfo: { name: "opensymphony-codex-benchmark", version: "0.0.0" },
      capabilities: {},
    });
    assertJsonRpcResult("websocket initialize", initialize.response);

    const batchStartedAt = performance.now();
    const requests = [];
    let nextRequestId = 2;
    for (let i = 0; i < iterations; i += 1) {
      requests.push(client.request(nextRequestId, "thread/loaded/list", { limit: 1 }));
      nextRequestId += 1;
    }
    const responses = await Promise.all(requests);
    for (const response of responses) {
      assertJsonRpcResult("websocket queued request", response.response);
    }
    const elapsedMs = performance.now() - batchStartedAt;
    const latencies = responses.map((response) => response.latencyMs);
    const requestsPerSecond =
      elapsedMs > 0 ? Number(((responses.length / elapsedMs) * 1000).toFixed(2)) : 0;

    const closed = waitForSocketClose(ws);
    client.dispose();
    client = null;
    ws.close();
    await closed;
    ws = null;
    const reconnectStartedAt = performance.now();
    ws2 = await openSocket(`ws://127.0.0.1:${port}`);
    reconnectClient = new WebSocketJsonRpcClient(ws2);
    const reconnectInitialize = await reconnectClient.request(nextRequestId, "initialize", {
      clientInfo: { name: "opensymphony-codex-benchmark-reconnect", version: "0.0.0" },
      capabilities: {},
    });
    assertJsonRpcResult("websocket reconnect initialize", reconnectInitialize.response);
    const reconnectMs = performance.now() - reconnectStartedAt;
    const { stdout, stderr } = decodeOutput();
    const stderrTrimmed = stderr.trim();

    return {
      transport: "websocket_loopback",
      port,
      initializeLatencyMs: Number(initialize.latencyMs.toFixed(3)),
      queuedRequests: iterations,
      queuedResponses: responses.length,
      queueElapsedMs: Number(elapsedMs.toFixed(3)),
      requestsPerSecond,
      latencyMs: {
        p50: percentile(latencies, 50),
        p95: percentile(latencies, 95),
        max: Number(latencies.reduce((max, latency) => Math.max(max, latency), 0).toFixed(3)),
      },
      reconnectLatencyMs: Number(reconnectMs.toFixed(3)),
      reconnectResponse: reconnectInitialize.response,
      stdoutBytes: Buffer.byteLength(stdout, "utf8"),
      stderrBytes: Buffer.byteLength(stderr, "utf8"),
      stderrPreview: stderrTrimmed ? [...stderrTrimmed].slice(-1000).join("") : null,
      exposure: {
        listener: `ws://127.0.0.1:${port}`,
        localhostOnly: /binds localhost only/.test(`${stdout}\n${stderr}`),
        authModesFromHelp: [
          ...(secureExposure.hasCapabilityTokenMode ? ["capability-token"] : []),
          ...(secureExposure.hasSignedBearerMode ? ["signed-bearer-token"] : []),
        ],
      },
    };
  } finally {
    if (client) client.dispose();
    if (reconnectClient) reconnectClient.dispose();
    if (ws && ws.readyState < WebSocket.CLOSING) ws.close();
    if (ws2 && ws2.readyState < WebSocket.CLOSING) ws2.close();
    await terminateChild(child);
  }
}

async function runHelpProbe() {
  const child = trackChild(
    spawn("codex", ["app-server", "--help"], {
      stdio: ["ignore", "pipe", "pipe"],
    }),
  );
  const help = await collectChildOutput(child, "codex app-server --help");
  return {
    transport: "websocket_secure_exposure",
    authEvidence: "advertised_in_help",
    helpSha256: createHash("sha256").update(help).digest("hex"),
    hasCapabilityTokenMode: help.includes("capability-token"),
    hasSignedBearerMode: help.includes("signed-bearer-token"),
    hasTokenFileFlag: help.includes("--ws-token-file"),
    hasTokenSha256Flag: help.includes("--ws-token-sha256"),
    hasSharedSecretFlag: help.includes("--ws-shared-secret-file"),
    hasIssuerFlag: help.includes("--ws-issuer"),
    hasAudienceFlag: help.includes("--ws-audience"),
    hasClockSkewFlag: help.includes("--ws-max-clock-skew-seconds"),
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
  const version = trackChild(spawn("codex", ["--version"], { stdio: ["ignore", "pipe", "pipe"] }));
  report.codexVersion = (await collectChildOutput(version, "codex --version")).trim();
  report.stdio = await runStdioProbe();
  report.secureExposure = await runHelpProbe();
  if (runWebSocket) {
    assertWebSocketRuntime();
    report.websocket = await runWebSocketProbe(report.secureExposure);
  }
  console.log(JSON.stringify(report, null, 2));
} catch (error) {
  console.error(error.stack || String(error));
  process.exit(1);
}
