/**
 * Renderer benchmark harness.
 *
 * Measures frame rate, memory growth, and UI responsiveness
 * for the terminal renderer under various load conditions.
 */

import type { TerminalFrame } from "@opensymphony/gateway-schema";
import type { RenderMetrics, RendererConfig } from "./renderer.js";
import type { DecodedFrame } from "./decoder.js";
import type { ScrollbackBuffer } from "./scrollback.js";
import { createTerminalRenderer, TerminalRenderer } from "./renderer.js";
import {
  generateBurstFrames,
  generateRealisticSession,
  generateBurstySession,
} from "./fixtures/terminal-fixtures.js";

export interface BenchmarkResult {
  name: string;
  totalFrames: number;
  durationMs: number;
  averageFps: number;
  peakFps: number;
  minFps: number;
  averageMemoryBytes: number;
  peakMemoryBytes: number;
  averageDecodeTimeMs: number;
  averageRenderTimeMs: number;
  uiBlockedFrames: number;
  uiBlockedPercent: number;
  metricsHistory: RenderMetrics[];
}

export interface BenchmarkConfig {
  warmupFrames: number;
  testFrames: number;
  maxDurationMs: number;
}

const DEFAULT_BENCHMARK_CONFIG: BenchmarkConfig = {
  warmupFrames: 100,
  testFrames: 1000,
  maxDurationMs: 30000,
};

/**
 * Run a benchmark test on the terminal renderer.
 */
export async function runBenchmark(
  name: string,
  frames: TerminalFrame[],
  config: Partial<BenchmarkConfig> = {},
): Promise<BenchmarkResult> {
  const mergedConfig = { ...DEFAULT_BENCHMARK_CONFIG, ...config };
  const renderer = createTerminalRenderer();
  const metricsHistory: RenderMetrics[] = [];
  let startTime = 0;
  let totalProcessed = 0;

  // Set up render callback to collect metrics
  renderer.onRender((decodedFrames: DecodedFrame[], _buffer: ScrollbackBuffer) => {
    totalProcessed += decodedFrames.length;
    const metrics = renderer.getMetrics();
    metricsHistory.push({ ...metrics, frameCount: totalProcessed });
  });

  // Warmup phase
  const warmupFrames = frames.slice(0, mergedConfig.warmupFrames);
  if (warmupFrames.length > 0) {
    for (const frame of warmupFrames) {
      renderer.queueFrame(frame.content, frame.encoding, frame);
    }
    // Allow warmup to complete
    await delay(100);
  }

  // Reset metrics after warmup
  metricsHistory.length = 0;
  totalProcessed = 0;
  startTime = performance.now();

  // Test phase — cap by testFrames and maxDurationMs
  const maxTest = mergedConfig.testFrames;
  const allTestFrames = frames.slice(mergedConfig.warmupFrames);
  const testFrames = allTestFrames.slice(0, maxTest);
  const deadline = startTime + mergedConfig.maxDurationMs;
  for (const frame of testFrames) {
    if (performance.now() > deadline) break;

    renderer.queueFrame(frame.content, frame.encoding, frame);

    // Sample metrics periodically
    if (metricsHistory.length % 10 === 0) {
      await delay(0); // Yield to allow rendering
    }
  }

  // Wait for all frames to be processed
  await delay(200);

  const duration = performance.now() - startTime;

  // Calculate results
  const result = calculateBenchmarkResult(
    name,
    testFrames.length,
    duration,
    metricsHistory,
  );

  // Cleanup
  renderer.dispose();

  return result;
}

/**
 * Run a burst benchmark test.
 */
export async function runBurstBenchmark(
  burstCount: number,
  config: Partial<BenchmarkConfig> = {},
): Promise<BenchmarkResult> {
  const frames = generateBurstFrames(burstCount, { includeAnsiCodes: true });
  return runBenchmark(`Burst ${burstCount} frames`, frames, config);
}

/**
 * Run a realistic session benchmark.
 */
export async function runRealisticSessionBenchmark(
  durationMs = 5000,
  fps = 30,
  config: Partial<BenchmarkConfig> = {},
): Promise<BenchmarkResult> {
  const frames = generateRealisticSession(durationMs, fps);
  return runBenchmark(`Realistic session (${durationMs}ms @ ${fps}fps)`, frames, config);
}

/**
 * Run a bursty session benchmark.
 */
export async function runBurstySessionBenchmark(
  durationMs = 5000,
  config: Partial<BenchmarkConfig> = {},
): Promise<BenchmarkResult> {
  const frames = generateBurstySession(durationMs);
  return runBenchmark(`Bursty session (${durationMs}ms)`, frames, config);
}

/**
 * Run all standard benchmarks and return results.
 */
export async function runAllBenchmarks(
  config: Partial<BenchmarkConfig> = {},
): Promise<BenchmarkResult[]> {
  const results: BenchmarkResult[] = [];

  // Burst benchmarks
  results.push(await runBurstBenchmark(100, config));
  results.push(await runBurstBenchmark(500, config));
  results.push(await runBurstBenchmark(1000, config));

  // Session benchmarks
  results.push(await runRealisticSessionBenchmark(2000, 30, config));
  results.push(await runBurstySessionBenchmark(3000, config));

  return results;
}

/**
 * Calculate benchmark result from metrics history.
 */
function calculateBenchmarkResult(
  name: string,
  totalFrames: number,
  durationMs: number,
  metricsHistory: RenderMetrics[],
): BenchmarkResult {
  if (metricsHistory.length === 0) {
    return {
      name,
      totalFrames,
      durationMs,
      averageFps: 0,
      peakFps: 0,
      minFps: 0,
      averageMemoryBytes: 0,
      peakMemoryBytes: 0,
      averageDecodeTimeMs: 0,
      averageRenderTimeMs: 0,
      uiBlockedFrames: 0,
      uiBlockedPercent: 0,
      metricsHistory: [],
    };
  }

  const fpsValues = metricsHistory.map((m) => m.fps).filter((f) => f > 0);
  const memoryValues = metricsHistory.map((m) => m.memoryBytes);
  const decodeTimes = metricsHistory.map((m) => m.decodeTimeMs);
  const renderTimes = metricsHistory.map((m) => m.renderTimeMs);
  const blockedCount = metricsHistory.filter((m) => m.uiBlocked).length;

  return {
    name,
    totalFrames,
    durationMs,
    averageFps: average(fpsValues) || 0,
    peakFps: Math.max(...fpsValues, 0),
    minFps: Math.min(...fpsValues, 0) || 0,
    averageMemoryBytes: average(memoryValues),
    peakMemoryBytes: Math.max(...memoryValues, 0),
    averageDecodeTimeMs: average(decodeTimes),
    averageRenderTimeMs: average(renderTimes),
    uiBlockedFrames: blockedCount,
    uiBlockedPercent: (blockedCount / metricsHistory.length) * 100,
    metricsHistory,
  };
}

/**
 * Calculate average of an array.
 */
function average(values: number[]): number {
  if (values.length === 0) return 0;
  return values.reduce((sum, v) => sum + v, 0) / values.length;
}

/**
 * Delay utility for async operations.
 */
function delay(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

/**
 * Format benchmark result as a human-readable report.
 */
export function formatBenchmarkReport(result: BenchmarkResult): string {
  return `
Benchmark: ${result.name}
----------------------------------------
Total Frames:      ${result.totalFrames}
Duration:          ${result.durationMs.toFixed(2)}ms
Average FPS:       ${result.averageFps.toFixed(1)}
Peak FPS:          ${result.peakFps}
Min FPS:           ${result.minFps}
Average Memory:    ${formatBytes(result.averageMemoryBytes)}
Peak Memory:       ${formatBytes(result.peakMemoryBytes)}
Avg Decode Time:   ${result.averageDecodeTimeMs.toFixed(2)}ms
Avg Render Time:   ${result.averageRenderTimeMs.toFixed(2)}ms
UI Blocked:        ${result.uiBlockedFrames} frames (${result.uiBlockedPercent.toFixed(2)}%)
----------------------------------------
  `.trim();
}

/**
 * Format bytes as human-readable string.
 */
function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(2)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(2)} MB`;
}

/**
 * Print all benchmark results.
 */
export function printBenchmarkResults(
  results: BenchmarkResult[],
  logger: (msg: string) => void = console.log,
): void {
  logger("=== Terminal Renderer Benchmark Results ===\n");

  for (const result of results) {
    logger(formatBenchmarkReport(result));
    logger("\n");
  }
}
