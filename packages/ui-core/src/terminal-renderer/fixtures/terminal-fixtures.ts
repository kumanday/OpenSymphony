/**
 * Fixture payload generator for bursty terminal and log output.
 *
 * Creates representative TerminalFrame payloads that simulate
 * high-throughput OpenHands output patterns for testing the renderer.
 */

import type { TerminalFrame, TerminalFrameKind, TerminalEncoding, SchemaVersion, TerminalLogAssociation } from "@opensymphony/gateway-schema";

const DEFAULT_ASSOCIATION: TerminalLogAssociation = {
  run_id: "fixture-run-1",
  workspace_id: "fixture-workspace-1",
};

export interface FixtureConfig {
  burstCount: number;
  burstDelayMs: number;
  frameSizeBytes: number;
  frameKind: TerminalFrameKind;
  encoding: TerminalEncoding;
  includeAnsiCodes: boolean;
  association?: TerminalLogAssociation;
}

const DEFAULT_CONFIG: FixtureConfig = {
  burstCount: 1000,
  burstDelayMs: 10,
  frameSizeBytes: 80,
  frameKind: "stdout",
  encoding: "utf8",
  includeAnsiCodes: false,
};

/**
 * Generate a single terminal frame for testing.
 */
export function createTerminalFrame(
  config: Partial<FixtureConfig> = {},
  sequence: number,
  runId = "fixture-run-1",
  sessionId = "fixture-session-1",
): TerminalFrame {
  const mergedConfig = { ...DEFAULT_CONFIG, ...config };

  return {
    schema_version: { major: 1, minor: 0, patch: 0 } as SchemaVersion,
    frame_sequence: sequence,
    stream_id: `stream-${sessionId}`,
    run_id: runId,
    terminal_session_id: sessionId,
    frame_kind: mergedConfig.frameKind,
    encoding: mergedConfig.encoding,
    content: generateFrameContent(mergedConfig),
    timestamp: new Date(Date.now() + sequence * mergedConfig.burstDelayMs).toISOString(),
    association: mergedConfig.association ?? { ...DEFAULT_ASSOCIATION, run_id: runId },
  };
}

/**
 * Generate frame content based on configuration.
 */
function generateFrameContent(config: FixtureConfig): string {
  let content = "";

  if (config.includeAnsiCodes) {
    content += generateAnsiText(config.frameSizeBytes);
  } else {
    content = generatePlainText(config.frameSizeBytes);
  }

  if (config.encoding === "base64") {
    content = btoa(content);
  }

  return content;
}

/**
 * Generate plain text content of approximately the given size.
 */
function generatePlainText(sizeBytes: number): string {
  const lorem = "Lorem ipsum dolor sit amet, consectetur adipiscing elit. ";
  let result = "";

  while (result.length < sizeBytes) {
    result += lorem;
  }

  return result.slice(0, sizeBytes) + "\n";
}

/**
 * Generate text content with ANSI escape codes.
 */
function generateAnsiText(sizeBytes: number): string {
  const colors = [
    "\x1b[31m", // red
    "\x1b[32m", // green
    "\x1b[33m", // yellow
    "\x1b[34m", // blue
    "\x1b[35m", // magenta
    "\x1b[36m", // cyan
    "\x1b[90m", // bright black
    "\x1b[91m", // bright red
  ];
  const reset = "\x1b[0m";

  let result = "";
  let colorIndex = 0;

  while (result.length < sizeBytes) {
    const color = colors[colorIndex % colors.length];
    const text = `Line ${Math.floor(result.length / 40)} `;
    result += color + text + reset;
    colorIndex++;
  }

  return result.slice(0, sizeBytes) + "\n";
}

/**
 * Generate a burst of terminal frames to simulate high-throughput output.
 * Returns an array of frames that can be fed to the renderer.
 */
export function generateBurstFrames(
  count: number,
  config: Partial<FixtureConfig> = {},
): TerminalFrame[] {
  const frames: TerminalFrame[] = [];

  for (let i = 0; i < count; i++) {
    frames.push(createTerminalFrame(config, i));
  }

  return frames;
}

/**
 * Generate a realistic log session with mixed frame types.
 * Simulates OpenHands agent output pattern.
 */
export function generateRealisticSession(
  durationMs = 60000,
  framesPerSecond = 30,
): TerminalFrame[] {
  const frames: TerminalFrame[] = [];
  const totalFrames = Math.floor((durationMs / 1000) * framesPerSecond);
  const runId = `session-${Date.now()}`;
  const sessionId = `term-${Date.now()}`;

  for (let i = 0; i < totalFrames; i++) {
    const timestamp = new Date(Date.now() + i * (1000 / framesPerSecond)).toISOString();

    // Mix of stdout and stderr frames
    const frameKind: TerminalFrameKind = i % 10 === 0 ? "stderr" : "stdout";

    frames.push({
      schema_version: { major: 1, minor: 0, patch: 0 } as SchemaVersion,
      frame_sequence: i,
      stream_id: `stream-${sessionId}`,
      run_id: runId,
      terminal_session_id: sessionId,
      frame_kind: frameKind,
      encoding: "utf8",
      content: generateRealisticLine(i, frameKind),
      timestamp,
      association: { ...DEFAULT_ASSOCIATION, run_id: runId },
    });
  }

  return frames;
}

/**
 * Generate a realistic terminal output line.
 * Uses a deterministic timestamp based on index for reproducible fixtures.
 */
function generateRealisticLine(index: number, kind: TerminalFrameKind, baseTimestampMs = 0): string {
  const stdoutMessages = [
    "Processing request...",
    "Building project...",
    "Running tests...",
    "Compiling source files...",
    "Loading configuration...",
    "Starting server...",
    "Listening on port 8080...",
    "Connection established.",
    "Data received successfully.",
    "Operation completed.",
  ];

  const stderrMessages = [
    "Warning: deprecated API usage detected",
    "Error: connection timeout",
    "Warning: memory usage high",
    "Error: invalid response format",
    "Warning: rate limit approaching",
  ];

  const messages = kind === "stderr" ? stderrMessages : stdoutMessages;
  const message = messages[index % messages.length];
  const ts = new Date(baseTimestampMs + index * 10).toISOString().substring(11, 23);

  return `[${ts}] ${message} (iteration ${index})\n`;
}

/**
 * Simple seeded pseudo-random number generator (LCG).
 * Produces deterministic output for reproducible test fixtures.
 */
function createSeededRandom(seed: number): () => number {
  let s = seed >>> 0; // Ensure unsigned 32-bit
  return () => {
    s = (s * 1664525 + 1013904223) >>> 0; // LCG parameters
    return s / 0x100000000; // Normalize to [0, 1)
  };
}

/**
 * Generate bursty output with varying rates.
 * Simulates realistic OpenHands agent behavior with spikes.
 * Uses a seeded PRNG for deterministic, reproducible output.
 */
export function generateBurstySession(
  totalDurationMs = 30000,
  burstIntervalMs = 2000,
  burstSize = 100,
  quietSize = 5,
  seed = 42,
): TerminalFrame[] {
  const frames: TerminalFrame[] = [];
  const runId = `burst-session-${seed}`;
  const sessionId = `burst-term-${seed}`;
  const rng = createSeededRandom(seed);
  let sequence = 0;

  for (let t = 0; t < totalDurationMs; t += burstIntervalMs) {
    // Determine if this is a burst or quiet period
    const isBurst = rng() < 0.3; // 30% chance of burst
    const count = isBurst ? burstSize : quietSize;

    for (let i = 0; i < count; i++) {
      const timestamp = new Date(t + (i * burstIntervalMs) / count).toISOString();

      frames.push({
        schema_version: { major: 1, minor: 0, patch: 0 } as SchemaVersion,
        frame_sequence: sequence++,
        stream_id: `stream-${sessionId}`,
        run_id: runId,
        terminal_session_id: sessionId,
        frame_kind: isBurst ? "stdout" : "log",
        encoding: "utf8",
        content: generateRealisticLine(sequence, isBurst ? "stdout" : "log"),
        timestamp,
        association: { ...DEFAULT_ASSOCIATION, run_id: runId },
      });
    }
  }

  return frames;
}

/**
 * Export fixture payloads as JSON for external consumption.
 */
export function exportFixturesToJson(
  frames: TerminalFrame[],
): string {
  return JSON.stringify(frames, null, 2);
}

/**
 * Load fixtures from JSON string.
 */
export function loadFixturesFromJson(json: string): TerminalFrame[] {
  return JSON.parse(json) as TerminalFrame[];
}
