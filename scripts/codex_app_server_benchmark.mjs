#!/usr/bin/env node

import { spawn } from "node:child_process";
import { createHash } from "node:crypto";
import { once, setMaxListeners } from "node:events";
import { setTimeout as sleep } from "node:timers/promises";

const booleanFlags = new Set(["--skip-websocket"]);
const valueFlags = new Set([
  "--batch-timeout-ms",
  "--codex-path",
  "--iterations",
  "--port",
  "--request-timeout-ms",
]);
const args = new Map();
for (let i = 2; i < process.argv.length; i += 1) {
  const rawArg = process.argv[i];
  if (!rawArg.startsWith("--")) continue;
  const equalsIndex = rawArg.indexOf("=");
  const arg = equalsIndex >= 0 ? rawArg.slice(0, equalsIndex) : rawArg;
  const inlineValue = equalsIndex >= 0 ? rawArg.slice(equalsIndex + 1) : null;
  if (booleanFlags.has(arg)) {
    args.set(arg, "true");
    continue;
  }
  if (!valueFlags.has(arg)) {
    console.error(`unknown option ${arg}`);
    process.exit(1);
  }
  if (inlineValue != null) {
    if (inlineValue.length === 0) {
      console.error(`${arg} requires a non-empty value`);
      process.exit(1);
    }
    args.set(arg, inlineValue);
    continue;
  }
  const next = process.argv[i + 1];
  if (!next || next.startsWith("--")) {
    console.error(`${arg} requires a value`);
    process.exit(1);
  }
  args.set(arg, next);
  i += 1;
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
const runWebSocket = !args.has("--skip-websocket");
const requestTimeoutMs = parseIntegerOption("--request-timeout-ms", 5000, 1, 300000);
const batchTimeoutMs = parseIntegerOption(
  "--batch-timeout-ms",
  Math.min(300000, requestTimeoutMs + iterations * 100),
  1,
  300000,
);
const codexPath = args.get("--codex-path") ?? "codex";
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

function websocketBatchTimeoutMs() {
  return batchTimeoutMs;
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
  if (Object.hasOwn(response, "error")) {
    if (response.error == null || typeof response.error !== "object") {
      throw new Error(`${label} returned malformed JSON-RPC error: ${JSON.stringify(response.error)}`);
    }
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
      if (this.stream.readableEnded || this.stream.destroyed) {
        throw new Error("stream ended before a complete line");
      }
      const remainingMs = Math.max(1, deadline - performance.now());
      const event = await waitForStreamProgress(this.stream, remainingMs);
      if (event === "timeout") throw new Error("timed out waiting for line");
      if (event === "end" || event === "close") {
        throw new Error(`stream ${event} before a complete line`);
      }
    }
  }
}

async function readJsonRpcResponse(reader, label, expectedId, timeoutMs) {
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
      if (String(response.id) === String(expectedId)) return response;
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

  const closePromise =
    child.exitCode !== null &&
    child.stdout?.readableEnded !== false &&
    child.stderr?.readableEnded !== false
      ? Promise.resolve({ code: child.exitCode, signal: child.signalCode, timedOut: false })
      : once(child, "close").then(([code, signal]) => ({ code, signal, timedOut: false }));
  const result = await Promise.race([
    closePromise,
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
    spawn(codexPath, ["app-server", "--stdio"], {
      stdio: ["pipe", "pipe", "pipe"],
    }),
  );
  let shuttingDown = false;
  try {
    child.stdout.setEncoding("utf8");
    child.stderr.setEncoding("utf8");
    let stderr = "";
    child.stderr.on("data", (chunk) => {
      stderr += chunk.toString("utf8");
    });
    const childFailure =
      child.exitCode !== null || child.signalCode !== null
        ? Promise.reject(
            new Error(
              `${codexPath} app-server --stdio exited before initialize completed with ${
                child.signalCode ? `signal ${child.signalCode}` : `code ${child.exitCode}`
              }${stderr.trim() ? `: ${stderr.trim()}` : ""}`,
            ),
          )
        : new Promise((_, reject) => {
            child.once("error", (error) => {
              reject(new Error(`${codexPath} app-server --stdio failed to start: ${error.message}`));
            });
            child.once("exit", (code, signal) => {
              if (shuttingDown) return;
              const suffix = signal ? `signal ${signal}` : `code ${code}`;
              reject(
                new Error(
                  `${codexPath} app-server --stdio exited before initialize completed with ${suffix}${stderr.trim() ? `: ${stderr.trim()}` : ""}`,
                ),
              );
            });
          });
    childFailure.catch(() => {});
    const withChildFailure = (promise) => {
      promise.catch(() => {});
      return Promise.race([promise, childFailure]);
    };
    const stdout = new LineReader(child.stdout);
    const startedAt = performance.now();
    await withChildFailure(
      writeToStream(
        child.stdin,
        `${request(1, "initialize", {
          clientInfo: { name: "opensymphony-codex-benchmark", version: "0.0.0" },
          capabilities: {},
        })}\n`,
        "stdio initialize",
        requestTimeoutMs,
      ),
    );
    const response = await withChildFailure(
      readJsonRpcResponse(stdout, "stdio initialize", 1, requestTimeoutMs),
    );
    const latencyMs = performance.now() - startedAt;
    assertJsonRpcResult("stdio initialize", response);
    shuttingDown = true;
    await endStream(child.stdin, "stdio initialize", requestTimeoutMs).catch(() => {});
    return {
      transport: "stdio",
      initializeLatencyMs: Number(latencyMs.toFixed(3)),
      response,
      stderrBytes: Buffer.byteLength(stderr, "utf8"),
    };
  } finally {
    shuttingDown = true;
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
      ws.close();
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
    if (!Object.hasOwn(parsed, "result") && !Object.hasOwn(parsed, "error")) {
      pending.reject(new Error(`malformed JSON-RPC response for ${pending.method}: ${JSON.stringify(parsed)}`));
      return;
    }
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
    spawn(codexPath, ["app-server", "--listen", `ws://127.0.0.1:${port}`], {
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
  let shuttingDown = false;
  const childFailure =
    child.exitCode !== null || child.signalCode !== null
      ? Promise.reject(
          new Error(
            `${codexPath} app-server exited unexpectedly during WebSocket probe with ${
              child.signalCode ? `signal ${child.signalCode}` : `code ${child.exitCode}`
            }; stdout=${JSON.stringify(decodeOutput().stdout.trim())}; stderr=${JSON.stringify(decodeOutput().stderr.trim())}`,
          ),
        )
      : new Promise((_, reject) => {
          child.once("error", (error) => {
            reject(new Error(`${codexPath} app-server failed during WebSocket probe: ${error.message}`));
          });
          child.once("exit", (code, signal) => {
            if (shuttingDown) return;
            const { stdout, stderr } = decodeOutput();
            const suffix = signal ? `signal ${signal}` : `code ${code}`;
            reject(
              new Error(
                `${codexPath} app-server exited unexpectedly during WebSocket probe with ${suffix}; stdout=${JSON.stringify(stdout.trim())}; stderr=${JSON.stringify(stderr.trim())}`,
              ),
            );
          });
        });
  childFailure.catch(() => {});
  const withChildFailure = (promise) => {
    promise.catch(() => {});
    return Promise.race([promise, childFailure]);
  };
  try {
    await withChildFailure(waitForReadyz(`http://127.0.0.1:${port}/readyz`, requestTimeoutMs));

    ws = await withChildFailure(openSocket(`ws://127.0.0.1:${port}`));
    client = new WebSocketJsonRpcClient(ws);
    const initialize = await withChildFailure(
      client.request(1, "initialize", {
        clientInfo: { name: "opensymphony-codex-benchmark", version: "0.0.0" },
        capabilities: {},
      }),
    );
    assertJsonRpcResult("websocket initialize", initialize.response);

    const batchStartedAt = performance.now();
    const requests = [];
    let nextRequestId = 2;
    const batchTimeoutMs = websocketBatchTimeoutMs();
    for (let i = 0; i < iterations; i += 1) {
      requests.push(
        withChildFailure(
          client.request(nextRequestId, "thread/loaded/list", { limit: 1 }, batchTimeoutMs),
        ),
      );
      nextRequestId += 1;
    }
    const settledResponses = await Promise.allSettled(requests);
    const failures = settledResponses.filter((response) => response.status === "rejected");
    if (failures.length > 0) {
      throw new Error(
        `websocket queued request batch failed (${failures.length}/${requests.length}): ${failures
          .map((failure) => failure.reason?.message ?? String(failure.reason))
          .join("; ")}`,
      );
    }
    const responses = settledResponses.map((response) => response.value);
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
    await withChildFailure(closed);
    ws = null;
    const reconnectStartedAt = performance.now();
    await withChildFailure(waitForReadyz(`http://127.0.0.1:${port}/readyz`, requestTimeoutMs));
    ws2 = await withChildFailure(openSocket(`ws://127.0.0.1:${port}`));
    reconnectClient = new WebSocketJsonRpcClient(ws2);
    const reconnectInitialize = await withChildFailure(
      reconnectClient.request(nextRequestId, "initialize", {
        clientInfo: { name: "opensymphony-codex-benchmark-reconnect", version: "0.0.0" },
        capabilities: {},
      }),
    );
    assertJsonRpcResult("websocket reconnect initialize", reconnectInitialize.response);
    const reconnectMs = performance.now() - reconnectStartedAt;
    const { stdout, stderr } = decodeOutput();
    const stderrTrimmed = stderr.trim();
    const output = `${stdout}\n${stderr}`;
    const configuredListener = `ws://127.0.0.1:${port}`;
    const listenerMatch = output.match(/\blistening on:\s*(ws:\/\/[^\s]+)/);
    const observedListener = listenerMatch?.[1] ?? configuredListener;
    const observedListenerSource = listenerMatch ? "observed" : "configured_fallback";
    let listenerHost = null;
    try {
      listenerHost = new URL(observedListener).hostname;
    } catch {
      listenerHost = null;
    }
    const localhostOnly = ["127.0.0.1", "localhost", "[::1]", "::1"].includes(listenerHost ?? "");

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
        listener: observedListener,
        observedListenerSource,
        listenerHost,
        localhostOnly,
        localhostOnlyEvidence: [
          "configured_loopback_listener",
          ...(listenerMatch ? ["parsed_listener_address"] : ["configured_listener_fallback"]),
        ],
        authEvidence: "advertised_in_help",
        runtimeAuthProbe: "not_measured_by_loopback_smoke",
        authModesAdvertisedInHelp: [
          ...(secureExposure.hasCapabilityTokenMode ? ["capability-token"] : []),
          ...(secureExposure.hasSignedBearerMode ? ["signed-bearer-token"] : []),
        ],
      },
    };
  } finally {
    shuttingDown = true;
    if (client) client.dispose();
    if (reconnectClient) reconnectClient.dispose();
    if (ws && ws.readyState < WebSocket.CLOSING) ws.close();
    if (ws2 && ws2.readyState < WebSocket.CLOSING) ws2.close();
    await terminateChild(child);
  }
}

async function runHelpProbe() {
  const child = trackChild(
    spawn(codexPath, ["app-server", "--help"], {
      stdio: ["ignore", "pipe", "pipe"],
    }),
  );
  const help = await collectChildOutput(child, `${codexPath} app-server --help`);
  const optionLinePattern = (flag) => new RegExp(`^\\s*(?:-\\w,\\s*)?${flag}\\b`);
  const optionHeaderPattern = /^\s*(?:-\w,\s*)?--[\w-]+\b/;
  const optionBlock = (flag) => {
    const lines = help.split(/\r?\n/);
    const start = lines.findIndex((line) => optionLinePattern(flag).test(line));
    if (start < 0) return "";
    const block = [];
    for (let i = start; i < lines.length; i += 1) {
      if (i > start && optionHeaderPattern.test(lines[i])) break;
      block.push(lines[i]);
    }
    return block.join("\n");
  };
  const optionLine = (flag) => optionBlock(flag).length > 0;
  const wsAuthBlock = optionBlock("--ws-auth");
  return {
    transport: "websocket_secure_exposure",
    authEvidence: "advertised_in_help",
    helpSha256: createHash("sha256").update(help).digest("hex"),
    hasCapabilityTokenMode: /\bcapability-token\b/.test(wsAuthBlock),
    hasSignedBearerMode: /\bsigned-bearer-token\b/.test(wsAuthBlock),
    hasTokenFileFlag: optionLine("--ws-token-file"),
    hasTokenSha256Flag: optionLine("--ws-token-sha256"),
    hasSharedSecretFlag: optionLine("--ws-shared-secret-file"),
    hasIssuerFlag: optionLine("--ws-issuer"),
    hasAudienceFlag: optionLine("--ws-audience"),
    hasClockSkewFlag: optionLine("--ws-max-clock-skew-seconds"),
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
  const version = trackChild(spawn(codexPath, ["--version"], { stdio: ["ignore", "pipe", "pipe"] }));
  report.codexVersion = (await collectChildOutput(version, `${codexPath} --version`)).trim();
  report.stdio = await runStdioProbe();
  report.secureExposure = await runHelpProbe();
  if (runWebSocket) {
    assertWebSocketRuntime();
    report.websocket = await runWebSocketProbe(report.secureExposure);
  }
  console.log(JSON.stringify(report, null, 2));
} catch (error) {
  console.error(error.stack || String(error));
  if (report.codexVersion || report.stdio || report.websocket || report.secureExposure) {
    console.error("Partial benchmark report:");
    console.error(JSON.stringify(report, null, 2));
  }
  process.exit(1);
}
