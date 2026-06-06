/**
 * Terminal renderer module exports.
 *
 * Re-exports all terminal rendering components for use by
 * both web and desktop clients.
 */

// Decoder
export {
  decodeAnsiText,
  decodeFrame,
  decodeBatch,
} from "./decoder.js";
export type {
  StyleSpan,
  TextStyle,
  ColorStyle,
  DecodedFrame,
  DecoderRequest,
  DecoderResponse,
} from "./decoder.js";

// Renderer
export {
  TerminalRenderer,
  createTerminalRenderer,
} from "./renderer.js";
export type {
  RenderMetrics,
  RendererConfig,
} from "./renderer.js";

// Scrollback buffer
export {
  createScrollbackBuffer,
  appendFrames,
  scrollTo,
  jumpToLatest,
  searchText,
  copyFrameRange,
  estimateMemoryUsage,
} from "./scrollback.js";
export type {
  ScrollbackBuffer,
} from "./scrollback.js";

// Fixtures
export {
  createTerminalFrame,
  generateBurstFrames,
  generateRealisticSession,
  generateBurstySession,
  exportFixturesToJson,
  loadFixturesFromJson,
} from "./fixtures/terminal-fixtures.js";
export type {
  FixtureConfig,
} from "./fixtures/terminal-fixtures.js";

// Benchmark
export {
  runBenchmark,
  runBurstBenchmark,
  runRealisticSessionBenchmark,
  runBurstySessionBenchmark,
  runAllBenchmarks,
  formatBenchmarkReport,
  printBenchmarkResults,
} from "./benchmark.js";
export type {
  BenchmarkResult,
  BenchmarkConfig,
} from "./benchmark.js";
