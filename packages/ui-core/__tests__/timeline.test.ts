/**
 * @jest-environment jsdom
 *
 * Timeline renderer tests for COE-412 runtime timeline integration.
 */

import { renderTimeline, filterTimelineEntries, findTimelineEntryByEventId, findTimelineEntryByEntityId } from "@opensymphony/ui-core";
import { schemaVersionV1 } from "@opensymphony/gateway-schema";
import type { RunTimeline } from "@opensymphony/gateway-schema";

function makeTimeline(): RunTimeline {
  return {
    schema_version: schemaVersionV1(),
    run_id: "run-abc",
    generated_at: "2025-09-01T00:00:00Z",
    entries: [
      {
        entry_id: "entry-1",
        sequence_start: 1,
        sequence_end: 3,
        happened_at: "2025-09-01T00:00:01Z",
        kind: "phase",
        phase: "waiting_on_prior_turn",
        title: "Waiting on prior turn",
        summary: "Run is blocked until the previous turn releases.",
        event_ids: ["evt-1", "evt-2", "evt-3"],
        entity_refs: [{ kind: "run" as const, id: "run-abc" }],
        file_paths: [],
      },
      {
        entry_id: "entry-2",
        sequence_start: 4,
        sequence_end: 7,
        happened_at: "2025-09-01T00:00:05Z",
        kind: "command",
        command_id: "cmd-1",
        title: "Shell command",
        summary: "Executed ls -la",
        event_ids: ["evt-4", "evt-5", "evt-6", "evt-7"],
        entity_refs: [
          { kind: "run" as const, id: "run-abc" },
          { kind: "command" as const, id: "cmd-1", identifier: "ls -la" },
        ],
        file_paths: [],
      },
      {
        entry_id: "entry-3",
        sequence_start: 8,
        sequence_end: 8,
        happened_at: "2025-09-01T00:00:10Z",
        kind: "state",
        title: "Run stalled",
        summary: "No terminal or log activity for too long.",
        event_ids: ["evt-8"],
        entity_refs: [{ kind: "run" as const, id: "run-abc" }],
        file_paths: [],
        state_evidence: {
          phase: "stalled",
          stream: "silent",
          last_activity_at: "2025-09-01T00:00:05Z",
          stall_deadline_at: "2025-09-01T00:00:10Z",
          explanation: "No frames or log lines for 5 seconds.",
        },
      },
    ],
  };
}

describe("timeline renderer", () => {
  it("renders a grouped timeline list", () => {
    const timeline = makeTimeline();
    const root = renderTimeline(timeline);

    expect(root.className).toBe("run-timeline");
    expect(root.getAttribute("data-run-id")).toBe("run-abc");

    const list = root.querySelector("ol.timeline-entries");
    expect(list).not.toBeNull();
    expect(list!.children.length).toBe(3);
  });

  it("renders empty state when there are no entries", () => {
    const timeline: RunTimeline = {
      schema_version: schemaVersionV1(),
      run_id: "run-empty",
      generated_at: "2025-09-01T00:00:00Z",
      entries: [],
    };
    const root = renderTimeline(timeline);
    expect(root.querySelector("p.timeline-empty")?.textContent).toBe(
      "No timeline events for this run.",
    );
  });

  it("includes kind, title, summary, and time on each entry", () => {
    const timeline = makeTimeline();
    const root = renderTimeline(timeline);
    const entry = root.querySelector("li[data-entry-id='entry-2']");
    expect(entry).not.toBeNull();
    expect(entry!.querySelector(".timeline-entry-kind")?.textContent).toBe("command");
    expect(entry!.querySelector(".timeline-entry-title")?.textContent).toBe("Shell command");
    expect(entry!.querySelector(".timeline-entry-summary")?.textContent).toBe("Executed ls -la");
    expect(entry!.querySelector("time")?.getAttribute("datetime")).toBe("2025-09-01T00:00:05Z");
  });

  it("renders entity refs and state evidence when present", () => {
    const timeline = makeTimeline();
    const root = renderTimeline(timeline);

    const commandEntry = root.querySelector("li[data-entry-id='entry-2']");
    const refs = commandEntry!.querySelectorAll(".timeline-entry-refs li");
    expect(refs.length).toBe(2);
    expect(refs[0].textContent).toBe("run: run-abc");
    expect(refs[1].textContent).toBe("command: ls -la (cmd-1)");

    const stateEntry = root.querySelector("li[data-entry-id='entry-3']");
    const evidence = stateEntry!.querySelector(".timeline-entry-evidence");
    expect(evidence).not.toBeNull();
    expect(evidence!.querySelector("summary")?.textContent).toBe("Evidence: stalled / silent");
  });

  it("filters entries by kind", () => {
    const timeline = makeTimeline();
    const commands = filterTimelineEntries(timeline, ["command"]);
    expect(commands.length).toBe(1);
    expect(commands[0].entry_id).toBe("entry-2");

    const progressLike = filterTimelineEntries(timeline, ["phase", "state"]);
    expect(progressLike.length).toBe(2);
  });

  it("finds an entry by event id", () => {
    const timeline = makeTimeline();
    const entry = findTimelineEntryByEventId(timeline, "evt-5");
    expect(entry).toBeDefined();
    expect(entry!.entry_id).toBe("entry-2");
    expect(findTimelineEntryByEventId(timeline, "missing")).toBeUndefined();
  });

  it("finds an entry by entity id", () => {
    const timeline = makeTimeline();
    const entry = findTimelineEntryByEntityId(timeline, "cmd-1");
    expect(entry).toBeDefined();
    expect(entry!.entry_id).toBe("entry-2");
    expect(findTimelineEntryByEntityId(timeline, "no-such-entity")).toBeUndefined();
  });
});
