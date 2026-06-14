/**
 * @jest-environment jsdom
 *
 * Terminal viewer tests for COE-412 terminal/log association integration.
 */

import { createTerminalViewer, type TerminalViewer } from "../src/terminal-viewer.js";
import { createTerminalRenderer, createTerminalFrame } from "@opensymphony/ui-core";
import type { TerminalLogAssociation } from "@opensymphony/gateway-schema";

describe("TerminalViewer association", () => {
  let container: HTMLElement;
  let viewer: TerminalViewer;
  let renderer: ReturnType<typeof createTerminalRenderer>;
  const originalRaf = globalThis.requestAnimationFrame;

  beforeEach(() => {
    // Make the renderer loop deterministic under jsdom by replacing rAF with a setTimeout call.
    globalThis.requestAnimationFrame = (cb: FrameRequestCallback) => {
      return window.setTimeout(() => cb(performance.now()), 0) as unknown as number;
    };
    container = document.createElement("div");
    document.body.appendChild(container);
    renderer = createTerminalRenderer();
  });

  afterEach(() => {
    globalThis.requestAnimationFrame = originalRaf;
    if (viewer) {
      viewer.destroy();
    }
    renderer.dispose();
    container.remove();
  });

  it("destroy removes event listeners without throwing", () => {
    viewer = createTerminalViewer(renderer, { container });

    // Calling destroy twice should be safe and listeners should be removed on the first call.
    expect(() => viewer.destroy()).not.toThrow();
    expect(() => viewer.destroy()).not.toThrow();
  });

  async function waitForRender(): Promise<void> {
    return new Promise((resolve) => setTimeout(resolve, 50));
  }

  it("displays association metadata in the status bar", async () => {
    viewer = createTerminalViewer(renderer, { container });

    const association: TerminalLogAssociation = {
      run_id: "run-abc",
      workspace_id: "ws-1",
      command_id: "cmd-1",
      issue_id: "issue-1",
      sub_issue_id: "sub-1",
    };
    viewer.setAssociationInfo(association);

    // Queueing a frame triggers a render, which refreshes the status bar.
    const frame = createTerminalFrame({ association }, 1);
    frame.source_event_id = "evt-1";
    renderer.queueFrame(frame.content, frame.encoding, frame);
    await waitForRender();

    const status = container.querySelector(".terminal-status-bar");
    expect(status).not.toBeNull();
    const text = status!.textContent ?? "";
    expect(text).toContain("run:run-abc");
    expect(text).toContain("cmd:cmd-1");
    expect(text).toContain("issue:issue-1");
    expect(text).toContain("sub:sub-1");
    expect(text).toContain("ws:ws-1");
  });

  it("renders frame lines with event id and sequence data attributes", async () => {
    viewer = createTerminalViewer(renderer, { container });

    const frame = createTerminalFrame(
      {
        association: { run_id: "run-abc", workspace_id: "ws-1" },
      },
      1,
    );
    frame.source_event_id = "evt-42";
    renderer.queueFrame(frame.content, frame.encoding, frame);
    await waitForRender();

    const line = container.querySelector("[data-event-id='evt-42']");
    expect(line).not.toBeNull();
    expect(line!.getAttribute("data-frame-sequence")).toBe("1");
  });

  it("jumps to the frame for a given source event id", async () => {
    viewer = createTerminalViewer(renderer, { container });

    for (let i = 1; i <= 5; i++) {
      const frame = createTerminalFrame(
        {
          association: { run_id: "run-abc", workspace_id: "ws-1" },
        },
        i,
      );
      frame.source_event_id = `evt-${i}`;
      renderer.queueFrame(frame.content, frame.encoding, frame);
    }
    await waitForRender();

    // We have 5 frames, all at the bottom. jumpToEvent returns false for unknown events.
    expect(viewer.jumpToEvent("evt-3")).toBe(true);
    expect(viewer.jumpToEvent("missing-event")).toBe(false);
  });
});
