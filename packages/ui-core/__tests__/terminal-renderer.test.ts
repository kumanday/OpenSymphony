/**
 * Terminal renderer tests.
 *
 * Covers decoder, scrollback buffer, renderer loop, and fixture generation.
 */

import {
  decodeAnsiText,
  decodeFrame,
  decodeBatch,
} from "../src/terminal-renderer/decoder.js";
import {
  createScrollbackBuffer,
  appendFrames,
  scrollTo,
  jumpToLatest,
  searchText,
  copyFrameRange,
  estimateMemoryUsage,
} from "../src/terminal-renderer/scrollback.js";
import {
  createTerminalFrame,
  generateBurstFrames,
  generateRealisticSession,
  generateBurstySession,
  exportFixturesToJson,
  loadFixturesFromJson,
} from "../src/terminal-renderer/fixtures/terminal-fixtures.js";
import {
  TerminalRenderer,
  createTerminalRenderer,
} from "../src/terminal-renderer/renderer.js";

// -- Decoder tests --

describe("decodeAnsiText", () => {
  it("strips ANSI codes and returns clean text", () => {
    const input = "\x1b[31mRed text\x1b[0m";
    const result = decodeAnsiText(input);
    expect(result.text).toBe("Red text");
    expect(result.spans.length).toBeGreaterThan(0);
  });

  it("handles plain text without ANSI codes", () => {
    const input = "Plain text without formatting";
    const result = decodeAnsiText(input);
    expect(result.text).toBe("Plain text without formatting");
    expect(result.spans).toEqual([]);
  });

  it("handles multiple ANSI codes in sequence", () => {
    const input = "\x1b[31mRed\x1b[32mGreen\x1b[34mBlue\x1b[0m";
    const result = decodeAnsiText(input);
    expect(result.text).toBe("RedGreenBlue");
    // 3 color codes + 1 reset code = 4 spans
    expect(result.spans.length).toBe(4);
  });

  it("handles empty string", () => {
    const result = decodeAnsiText("");
    expect(result.text).toBe("");
    expect(result.spans).toEqual([]);
  });

  it("handles newlines correctly", () => {
    const input = "Line 1\n\x1b[31mLine 2\x1b[0m\nLine 3";
    const result = decodeAnsiText(input);
    expect(result.text).toBe("Line 1\nLine 2\nLine 3");
  });
});

describe("decodeFrame", () => {
  it("decodes utf8 content", () => {
    const frame = createTerminalFrame({}, 1);
    const result = decodeFrame("Hello World", "utf8", frame);
    expect(result.text).toBe("Hello World");
    expect(result.frame).toBe(frame);
    expect(result.decodedAt).toBeGreaterThan(0);
  });

  it("decodes base64 content", () => {
    const frame = createTerminalFrame({}, 1);
    const content = btoa("Hello World");
    const result = decodeFrame(content, "base64", frame);
    expect(result.text).toBe("Hello World");
  });

  it("handles invalid base64 gracefully", () => {
    const frame = createTerminalFrame({}, 1);
    const result = decodeFrame("not-valid-base64!!!", "base64", frame);
    expect(result.text).toBe("[decode error: invalid base64]");
  });

  it("decodes ANSI-encoded content", () => {
    const frame = createTerminalFrame({ includeAnsiCodes: true }, 1);
    const result = decodeFrame(frame.content, "utf8", frame);
    expect(result.text.length).toBeGreaterThan(0);
    expect(result.spans.length).toBeGreaterThan(0);
  });
});

describe("decodeBatch", () => {
  it("decodes multiple frames efficiently", () => {
    const frames = generateBurstFrames(100);
    const payloads = frames.map((f) => ({
      content: f.content,
      encoding: f.encoding as "utf8" | "base64",
      frame: f,
    }));

    const results = decodeBatch(payloads);
    expect(results.length).toBe(100);
    expect(results.every((r) => r.decodedAt > 0)).toBe(true);
  });

  it("handles empty batch", () => {
    const results = decodeBatch([]);
    expect(results).toEqual([]);
  });
});

// -- Scrollback buffer tests --

describe("createScrollbackBuffer", () => {
  it("creates buffer with default capacity", () => {
    const buffer = createScrollbackBuffer();
    expect(buffer.capacity).toBe(1000);
    expect(buffer.visibleFrames).toEqual([]);
    expect(buffer.totalFrames).toBe(0);
    expect(buffer.atBottom).toBe(true);
  });

  it("creates buffer with custom capacity", () => {
    const buffer = createScrollbackBuffer(500);
    expect(buffer.capacity).toBe(500);
  });
});

describe("appendFrames", () => {
  it("appends frames to buffer", () => {
    const buffer = createScrollbackBuffer(100);
    const frames = generateBurstFrames(10).map((f) =>
      decodeFrame(f.content, f.encoding, f)
    );

    const result = appendFrames(buffer, frames);
    expect(result.totalFrames).toBe(10);
    expect(result.visibleFrames.length).toBe(10);
    expect(result.offset).toBe(0);
  });

  it("prunes old frames when capacity exceeded", () => {
    const buffer = createScrollbackBuffer(5);
    const frames1 = generateBurstFrames(3).map((f) =>
      decodeFrame(f.content, f.encoding, f)
    );
    const buffer2 = appendFrames(buffer, frames1);

    const frames2 = generateBurstFrames(4).map((f) =>
      decodeFrame(f.content, f.encoding, f)
    );
    const buffer3 = appendFrames(buffer2, frames2);

    expect(buffer3.totalFrames).toBe(7);
    expect(buffer3.visibleFrames.length).toBe(5); // Pruned to capacity
    expect(buffer3.offset).toBe(2); // First 2 frames pruned
  });

  it("handles appending empty array", () => {
    const buffer = createScrollbackBuffer(100);
    const result = appendFrames(buffer, []);
    expect(result).toBe(buffer); // No change
  });

  it("caps visible frames when a single append batch exceeds capacity", () => {
    const buffer = createScrollbackBuffer(5);
    const frames = generateBurstFrames(12).map((f) =>
      decodeFrame(f.content, f.encoding, f)
    );

    const result = appendFrames(buffer, frames);

    expect(result.totalFrames).toBe(12);
    expect(result.visibleFrames.length).toBe(5);
    expect(result.offset).toBe(7);
  });

  it("preserves the visible window while scrolled up", () => {
    const buffer = createScrollbackBuffer(5);
    const frames = generateBurstFrames(10).map((f) =>
      decodeFrame(f.content, f.encoding, f)
    );
    const filledBuffer = appendFrames(buffer, frames);
    const scrolledUp = scrollTo(filledBuffer, 2);
    const visibleBefore = scrolledUp.visibleFrames.map((frame) => frame.frame.frame_sequence);

    const newFrames = generateBurstFrames(3).map((f, index) => {
      const frame = { ...f, frame_sequence: 10 + index };
      return decodeFrame(frame.content, frame.encoding, frame);
    });
    const updated = appendFrames(scrolledUp, newFrames);

    expect(updated.atBottom).toBe(false);
    expect(updated.visibleFrames.map((frame) => frame.frame.frame_sequence)).toEqual(visibleBefore);
  });
});

describe("scrollTo", () => {
  it("scrolls to specific frame index", () => {
    const buffer = createScrollbackBuffer(10);
    const frames = generateBurstFrames(100).map((f) =>
      decodeFrame(f.content, f.encoding, f)
    );
    const filledBuffer = appendFrames(buffer, frames);

    const result = scrollTo(filledBuffer, 50);
    expect(result.atBottom).toBe(false);
    expect(result.offset).toBeGreaterThanOrEqual(0);
  });

  it("clamps target index to valid range", () => {
    const buffer = createScrollbackBuffer(10);
    const result1 = scrollTo(buffer, -5);
    expect(result1.offset).toBe(0);

    const result2 = scrollTo(buffer, 999999);
    expect(result2.offset).toBe(0);
  });

  it("returns different visible frames for different scroll positions", () => {
    const buffer = createScrollbackBuffer(10);
    const frames = generateBurstFrames(100).map((f) =>
      decodeFrame(f.content, f.encoding, f)
    );
    const filledBuffer = appendFrames(buffer, frames);

    const atBottom = filledBuffer.visibleFrames;
    const scrolledUp = scrollTo(filledBuffer, 20);

    // Scrolling up should show different frames
    expect(scrolledUp.visibleFrames.length).toBeLessThanOrEqual(buffer.capacity);
    // The scrolled-up view should start from a different offset
    expect(scrolledUp.offset).toBeLessThan(filledBuffer.offset);
  });
});

describe("jumpToLatest", () => {
  it("sets atBottom to true", () => {
    const buffer = createScrollbackBuffer(100);
    const result = jumpToLatest(buffer);
    expect(result.atBottom).toBe(true);
  });

  it("restores the latest visible window after scrolling up", () => {
    const buffer = createScrollbackBuffer(5);
    const frames = generateBurstFrames(20).map((f) =>
      decodeFrame(f.content, f.encoding, f)
    );
    const filledBuffer = appendFrames(buffer, frames);
    const scrolledUp = scrollTo(filledBuffer, 3);

    const result = jumpToLatest(scrolledUp);

    expect(result.atBottom).toBe(true);
    expect(result.offset).toBe(15);
    expect(result.visibleFrames.map((frame) => frame.frame.frame_sequence)).toEqual([15, 16, 17, 18, 19]);
  });
});

describe("searchText", () => {
  it("finds matching frames", () => {
    const buffer = createScrollbackBuffer(100);
    const frames = generateBurstFrames(10).map((f) =>
      decodeFrame(f.content, f.encoding, f)
    );
    const filledBuffer = appendFrames(buffer, frames);

    // Search for content that exists
    const results = searchText(filledBuffer, "Lorem");
    expect(results.length).toBeGreaterThan(0);
  });

  it("handles case-insensitive search", () => {
    const buffer = createScrollbackBuffer(100);
    const frames = generateBurstFrames(5).map((f) =>
      decodeFrame(f.content, f.encoding, f)
    );
    const filledBuffer = appendFrames(buffer, frames);

    const results = searchText(filledBuffer, "LOREM", false);
    expect(results.length).toBeGreaterThan(0);
  });

  it("returns empty array for no matches", () => {
    const buffer = createScrollbackBuffer(100);
    const frames = generateBurstFrames(5).map((f) =>
      decodeFrame(f.content, f.encoding, f)
    );
    const filledBuffer = appendFrames(buffer, frames);

    const results = searchText(filledBuffer, "nonexistent_string_xyz");
    expect(results).toEqual([]);
  });

  it("handles empty query", () => {
    const buffer = createScrollbackBuffer(100);
    const results = searchText(buffer, "");
    expect(results).toEqual([]);
  });

  it("returns global frame indices after history pruning", () => {
    const buffer = createScrollbackBuffer(5);
    const frames = generateBurstFrames(60).map((f, index) => {
      const frame = { ...f, content: `unique frame ${index}\n` };
      return decodeFrame(frame.content, frame.encoding, frame);
    });
    const filledBuffer = appendFrames(buffer, frames);

    const results = searchText(filledBuffer, "unique frame 42");

    expect(filledBuffer.allFrames.length).toBe(50);
    expect(results).toEqual([42]);
  });
});

describe("copyFrameRange", () => {
  it("copies text from frame range", () => {
    const buffer = createScrollbackBuffer(100);
    const frames = generateBurstFrames(10).map((f) =>
      decodeFrame(f.content, f.encoding, f)
    );
    const filledBuffer = appendFrames(buffer, frames);

    const text = copyFrameRange(filledBuffer, 0, 4);
    expect(text.length).toBeGreaterThan(0);
    expect(text).toContain("\n");
  });

  it("handles invalid range", () => {
    const buffer = createScrollbackBuffer(100);
    const text = copyFrameRange(buffer, 10, 5);
    expect(text).toBe("");
  });

  it("handles out-of-bounds range", () => {
    const buffer = createScrollbackBuffer(100);
    const text = copyFrameRange(buffer, 0, 999);
    expect(text).toBe("");
  });
});

describe("estimateMemoryUsage", () => {
  it("returns reasonable estimate", () => {
    const buffer = createScrollbackBuffer(100);
    const frames = generateBurstFrames(50).map((f) =>
      decodeFrame(f.content, f.encoding, f)
    );
    const filledBuffer = appendFrames(buffer, frames);

    const memory = estimateMemoryUsage(filledBuffer);
    expect(memory).toBeGreaterThan(0);
  });

  it("handles empty buffer", () => {
    const buffer = createScrollbackBuffer(100);
    const memory = estimateMemoryUsage(buffer);
    expect(memory).toBe(256); // Base overhead
  });
});

// -- Fixture generation tests --

describe("createTerminalFrame", () => {
  it("creates frame with default config", () => {
    const frame = createTerminalFrame({}, 1);
    expect(frame.frame_sequence).toBe(1);
    expect(frame.frame_kind).toBe("stdout");
    expect(frame.encoding).toBe("utf8");
    expect(frame.content.length).toBeGreaterThan(0);
  });

  it("creates frame with custom config", () => {
    const frame = createTerminalFrame(
      {
        frameKind: "stderr",
        includeAnsiCodes: true,
        frameSizeBytes: 120,
      },
      5
    );
    expect(frame.frame_kind).toBe("stderr");
    expect(frame.content).toContain("\x1b[");
  });

  it("creates base64 encoded frame", () => {
    const frame = createTerminalFrame({ encoding: "base64" }, 1);
    expect(frame.encoding).toBe("base64");
    // Verify it's valid base64
    expect(() => atob(frame.content)).not.toThrow();
  });
});

describe("generateBurstFrames", () => {
  it("generates specified number of frames", () => {
    const frames = generateBurstFrames(50);
    expect(frames.length).toBe(50);
  });

  it("generates frames with incrementing sequence numbers", () => {
    const frames = generateBurstFrames(10);
    frames.forEach((frame, index) => {
      expect(frame.frame_sequence).toBe(index);
    });
  });

  it("generates frames with ANSI codes when requested", () => {
    const frames = generateBurstFrames(5, { includeAnsiCodes: true });
    frames.forEach((frame) => {
      expect(frame.content).toContain("\x1b[");
    });
  });
});

describe("generateRealisticSession", () => {
  it("generates session with mixed frame types", () => {
    const frames = generateRealisticSession(1000, 30);
    expect(frames.length).toBe(30); // 1 second * 30 fps

    const hasStdout = frames.some((f) => f.frame_kind === "stdout");
    const hasStderr = frames.some((f) => f.frame_kind === "stderr");
    expect(hasStdout).toBe(true);
    expect(hasStderr).toBe(true);
  });

  it("generates frames with unique timestamps", () => {
    const frames = generateRealisticSession(2000, 10);
    const timestamps = frames.map((f) => f.timestamp);
    const uniqueTimestamps = new Set(timestamps);
    expect(uniqueTimestamps.size).toBe(timestamps.length);
  });
});

describe("generateBurstySession", () => {
  it("generates session with varying frame rates", () => {
    // Use parameters that guarantee both burst and quiet periods
    const frames = generateBurstySession(5000, 500, 50, 5);
    expect(frames.length).toBeGreaterThan(0);

    // With 10 iterations (5000/500) and 30% burst chance, we should get both types
    const stdoutCount = frames.filter((f) => f.frame_kind === "stdout").length;
    const logCount = frames.filter((f) => f.frame_kind === "log").length;
    expect(stdoutCount + logCount).toBe(frames.length);
  });

  it("produces deterministic output with same seed", () => {
    const session1 = generateBurstySession(5000, 500, 50, 5, 42);
    const session2 = generateBurstySession(5000, 500, 50, 5, 42);

    expect(session1.length).toBe(session2.length);
    for (let i = 0; i < session1.length; i++) {
      expect(session1[i].frame_kind).toBe(session2[i].frame_kind);
      expect(session1[i].frame_sequence).toBe(session2[i].frame_sequence);
      expect(session1[i].content).toBe(session2[i].content);
    }
  });

  it("produces different output with different seeds", () => {
    const session1 = generateBurstySession(5000, 500, 50, 5, 42);
    const session2 = generateBurstySession(5000, 500, 50, 5, 99);

    // At least some frames should differ
    const differs = session1.filter(
      (f, i) => session2[i]?.frame_kind !== f.frame_kind,
    ).length;
    expect(differs).toBeGreaterThan(0);
  });
});

describe("exportFixturesToJson", () => {
  it("exports frames to valid JSON", () => {
    const frames = generateBurstFrames(5);
    const json = exportFixturesToJson(frames);
    expect(() => JSON.parse(json)).not.toThrow();
  });

  it("roundtrips correctly", () => {
    const frames = generateBurstFrames(10);
    const json = exportFixturesToJson(frames);
    const loaded = loadFixturesFromJson(json);
    expect(loaded.length).toBe(frames.length);
  });
});

describe("loadFixturesFromJson", () => {
  it("loads valid JSON", () => {
    const json = JSON.stringify([
      {
        schema_version: { major: 1, minor: 0, patch: 0 },
        frame_sequence: 1,
        stream_id: "stream-1",
        run_id: "run-1",
        terminal_session_id: "term-1",
        frame_kind: "stdout",
        encoding: "utf8",
        content: "test",
        timestamp: "2025-01-01T00:00:00Z",
      },
    ]);

    const frames = loadFixturesFromJson(json);
    expect(frames.length).toBe(1);
    expect(frames[0].frame_kind).toBe("stdout");
  });

  it("throws on invalid JSON", () => {
    expect(() => loadFixturesFromJson("{invalid")).toThrow();
  });
});

// -- Renderer tests --

function queueGeneratedFrames(renderer: TerminalRenderer, startSequence: number, count: number): void {
  const frames = generateBurstFrames(count);
  for (let index = 0; index < frames.length; index++) {
    const frame = {
      ...frames[index],
      content: `frame ${startSequence + index}`,
      frame_sequence: startSequence + index,
    };
    renderer.queueFrame(frame.content, frame.encoding, frame);
  }
}

async function waitForFrameCount(renderer: TerminalRenderer, expectedCount: number): Promise<void> {
  const deadline = Date.now() + 2000;
  while (renderer.getMetrics().frameCount < expectedCount && Date.now() < deadline) {
    await new Promise((resolve) => setTimeout(resolve, 20));
  }
}

describe("TerminalRenderer", () => {
  it("creates renderer with default config", () => {
    const renderer = createTerminalRenderer();
    expect(renderer).toBeDefined();
  });

  it("queues frames without blocking", () => {
    const renderer = createTerminalRenderer();
    const frames = generateBurstFrames(100);

    // This should not throw or block
    for (const frame of frames) {
      renderer.queueFrame(frame.content, frame.encoding, frame);
    }

    // Give render loop time to process
    return new Promise<void>((resolve) => {
      setTimeout(() => {
        const metrics = renderer.getMetrics();
        expect(metrics.frameCount).toBe(100);
        renderer.dispose();
        resolve();
      }, 100);
    });
  }, 10000);

  it("reports metrics correctly", () => {
    const renderer = createTerminalRenderer();
    const metrics = renderer.getMetrics();

    expect(metrics).toHaveProperty("fps");
    expect(metrics).toHaveProperty("memoryBytes");
    expect(metrics).toHaveProperty("frameCount");
    expect(metrics).toHaveProperty("decodeTimeMs");
    expect(metrics).toHaveProperty("renderTimeMs");
    expect(metrics).toHaveProperty("uiBlocked");

    renderer.dispose();
  });

  it("handles clear operation", () => {
    const renderer = createTerminalRenderer();
    const frames = generateBurstFrames(50);

    for (const frame of frames) {
      renderer.queueFrame(frame.content, frame.encoding, frame);
    }

    renderer.clear();

    return new Promise<void>((resolve) => {
      setTimeout(() => {
        const metrics = renderer.getMetrics();
        expect(metrics.frameCount).toBe(0);
        renderer.dispose();
        resolve();
      }, 50);
    });
  }, 10000);

  it("notifies render subscribers when cleared", () => {
    const renderer = createTerminalRenderer();
    let cleared = false;

    renderer.onRender((frames, buffer) => {
      if (frames.length === 0 && buffer.totalFrames === 0) {
        cleared = true;
      }
    });

    renderer.clear();

    expect(cleared).toBe(true);
    renderer.dispose();
  });

  it("handles dispose correctly", () => {
    const renderer = createTerminalRenderer();
    const frames = generateBurstFrames(10);

    for (const frame of frames) {
      renderer.queueFrame(frame.content, frame.encoding, frame);
    }

    renderer.dispose();

    // Should not throw after dispose
    expect(() => renderer.getMetrics()).not.toThrow();
  });

  it("calls onRender callback", () => {
    const renderer = createTerminalRenderer();
    let renderCount = 0;

    renderer.onRender(() => {
      renderCount++;
    });

    const frames = generateBurstFrames(5);
    for (const frame of frames) {
      renderer.queueFrame(frame.content, frame.encoding, frame);
    }

    return new Promise<void>((resolve) => {
      setTimeout(() => {
        expect(renderCount).toBeGreaterThan(0);
        renderer.dispose();
        resolve();
      }, 100);
    });
  }, 10000);

  it("supports unsubscribing the render callback", () => {
    const renderer = createTerminalRenderer();
    let renderCount = 0;

    const unsubscribe = renderer.onRender(() => {
      renderCount++;
    });
    unsubscribe();

    const frames = generateBurstFrames(5);
    for (const frame of frames) {
      renderer.queueFrame(frame.content, frame.encoding, frame);
    }

    return new Promise<void>((resolve) => {
      setTimeout(() => {
        expect(renderCount).toBe(0);
        renderer.dispose();
        resolve();
      }, 100);
    });
  }, 10000);

  it("jumps to latest output", () => {
    const renderer = createTerminalRenderer();
    const buffer = renderer.getBuffer();

    renderer.jumpToLatest();

    const newBuffer = renderer.getBuffer();
    expect(newBuffer.atBottom).toBe(true);
  });

  it("preserves manually synced scroll position while live frames arrive", async () => {
    const renderer = createTerminalRenderer({
      maxBufferCapacity: 20,
      renderIntervalMs: 0,
      batchSize: 100,
    });

    try {
      queueGeneratedFrames(renderer, 0, 30);
      await waitForFrameCount(renderer, 30);

      renderer.syncScrollPosition(12, false);
      const scrolledBuffer = renderer.getBuffer();
      const visibleBefore = scrolledBuffer.visibleFrames.map(
        (frame) => frame.frame.frame_sequence,
      );

      queueGeneratedFrames(renderer, 30, 5);
      await waitForFrameCount(renderer, 35);

      const updatedBuffer = renderer.getBuffer();
      expect(updatedBuffer.atBottom).toBe(false);
      expect(updatedBuffer.offset).toBe(scrolledBuffer.offset);
      expect(updatedBuffer.visibleFrames.map((frame) => frame.frame.frame_sequence))
        .toEqual(visibleBefore);
    } finally {
      renderer.dispose();
    }
  });
});

// -- Performance tests --

describe("Performance", () => {
  it("handles burst of 1000 frames within time budget", async () => {
    const renderer = createTerminalRenderer();
    const frames = generateBurstFrames(1000, { includeAnsiCodes: true });

    try {
      // Measure only the time to queue frames (not wall-clock including idle)
      const start = performance.now();
      for (const frame of frames) {
        renderer.queueFrame(frame.content, frame.encoding, frame);
      }
      const queueTime = performance.now() - start;

      // Queueing should be fast and non-blocking.
      expect(queueTime).toBeLessThan(100);

      const deadline = Date.now() + 2000;
      while (renderer.getMetrics().frameCount < frames.length && Date.now() < deadline) {
        await new Promise((resolve) => setTimeout(resolve, 20));
      }

      const metrics = renderer.getMetrics();
      expect(metrics.frameCount).toBe(frames.length);
    } finally {
      renderer.dispose();
    }
  }, 10000);

  it("maintains stable memory usage under load", async () => {
    const renderer = createTerminalRenderer({ maxBufferCapacity: 500 });

    // Simulate sustained output
    for (let batch = 0; batch < 10; batch++) {
      const frames = generateBurstFrames(100);
      for (const frame of frames) {
        renderer.queueFrame(frame.content, frame.encoding, frame);
      }
      await new Promise((resolve) => setTimeout(resolve, 10));
    }

    // Wait for processing
    await new Promise<void>((resolve) => {
      setTimeout(() => {
        const metrics = renderer.getMetrics();
        const buffer = renderer.getBuffer();

        // Buffer should be capped at capacity
        expect(buffer.visibleFrames.length).toBeLessThanOrEqual(500);
        // Memory should be bounded
        expect(metrics.memoryBytes).toBeLessThan(500 * 1024); // Less than 500KB

        renderer.dispose();
        resolve();
      }, 100);
    });
  }, 10000);
});
