import type { MemoryCompletedTask } from "@opensymphony/gateway-schema";
import {
  createFixtureGraphAdapter,
  graphVizFixtureCompletedTasks,
  graphVizFixtureSnapshot,
  pageCompletedTasks,
} from "../src/index.js";

const rows: MemoryCompletedTask[] = [
  {
    issue_key: "COE-99",
    concept_id: "issues/COE-99",
    bundle_id: "local-default",
    title: "Ninety-nine",
    state: "Done",
    milestone: "M1",
    completed_at: "2026-06-01T10:00:00Z",
    prs: [
      { number: 120, title: "COE-99 fix", url: "https://example.com/pull/120", merged: true, merged_at: "2026-06-01T09:00:00Z" },
    ],
    source: "memory",
  },
  {
    issue_key: "COE-100",
    concept_id: "issues/COE-100",
    bundle_id: "local-default",
    title: "One hundred",
    state: "Done",
    milestone: "M2",
    completed_at: "2026-06-03T10:00:00Z",
    prs: [
      { number: 90, title: "COE-100 first attempt", url: "https://example.com/pull/90", merged: false },
      { number: 130, title: "COE-100 landed", url: "https://example.com/pull/130", merged: true, merged_at: "2026-06-03T09:00:00Z" },
    ],
    source: "memory",
  },
  {
    issue_key: "COE-101",
    concept_id: "",
    title: "Uncaptured completion",
    state: "Done",
    prs: [],
    source: "orchestrator",
  },
];

describe("pageCompletedTasks", () => {
  it("defaults to newest completion first with missing dates last", () => {
    const page = pageCompletedTasks(rows);
    expect(page.tasks.map((task) => task.issue_key)).toEqual(["COE-100", "COE-99", "COE-101"]);
    expect(page.sort).toBe("completed_desc");
    expect(page.total).toBe(3);
  });

  it("sorts issue keys naturally so COE-99 precedes COE-100", () => {
    const page = pageCompletedTasks(rows, { sort: "id_asc" });
    expect(page.tasks.map((task) => task.issue_key)).toEqual(["COE-99", "COE-100", "COE-101"]);
    const descending = pageCompletedTasks(rows, { sort: "id_desc" });
    expect(descending.tasks.map((task) => task.issue_key)).toEqual(["COE-101", "COE-100", "COE-99"]);
  });

  it("sorts by latest PR number", () => {
    const page = pageCompletedTasks(rows, { sort: "pr_desc" });
    expect(page.tasks.map((task) => task.issue_key)).toEqual(["COE-100", "COE-99", "COE-101"]);
  });

  it("searches issue key, title, milestone, and PR text", () => {
    expect(pageCompletedTasks(rows, { query: "hundred" }).tasks.map((task) => task.issue_key))
      .toEqual(["COE-100"]);
    expect(pageCompletedTasks(rows, { query: "m1" }).tasks.map((task) => task.issue_key))
      .toEqual(["COE-99"]);
    expect(pageCompletedTasks(rows, { query: "#130" }).tasks.map((task) => task.issue_key))
      .toEqual(["COE-100"]);
    expect(pageCompletedTasks(rows, { query: "uncaptured" }).tasks.map((task) => task.issue_key))
      .toEqual(["COE-101"]);
  });

  it("paginates with clamped offsets and reports totals", () => {
    const page = pageCompletedTasks(rows, { limit: 2, offset: 2, sort: "id_asc" });
    expect(page.total).toBe(3);
    expect(page.tasks.map((task) => task.issue_key)).toEqual(["COE-101"]);
    expect(page.offset).toBe(2);
    expect(page.limit).toBe(2);
    const beyond = pageCompletedTasks(rows, { limit: 2, offset: 99 });
    expect(beyond.tasks).toEqual([]);
    expect(beyond.offset).toBe(3);
  });

  it("falls back to the default sort for unknown sort keys", () => {
    expect(pageCompletedTasks(rows, { sort: "nonsense" }).sort).toBe("completed_desc");
  });
});

describe("fixture graph adapter completed tasks", () => {
  it("serves paginated fixture rows like the gateway endpoint", async () => {
    const adapter = createFixtureGraphAdapter({ completedTasks: graphVizFixtureCompletedTasks });
    const first = await adapter.getCompletedTasks!({ limit: 25 });
    expect(first.total).toBe(graphVizFixtureCompletedTasks.length);
    expect(first.tasks).toHaveLength(25);
    expect(first.tasks[0].issue_key).toBe("VIZ-100");
    const second = await adapter.getCompletedTasks!({ limit: 25, offset: 25 });
    expect(second.tasks.length).toBe(graphVizFixtureCompletedTasks.length - 25);
  });

  it("keeps deterministic fixture rows whose capsules resolve in the viz snapshot", () => {
    const conceptIds = new Set(
      graphVizFixtureSnapshot.nodes
        .filter((node) => node.kind === "concept")
        .map((node) => node.concept_id),
    );
    for (const task of graphVizFixtureCompletedTasks) {
      expect(conceptIds.has(task.concept_id)).toBe(true);
      expect(task.prs.length).toBeGreaterThan(0);
    }
    // The multi-PR presentation (bold newest, strike unmerged) has fixture
    // coverage: some tasks carry an earlier abandoned PR.
    const multiPr = graphVizFixtureCompletedTasks.filter((task) => task.prs.length > 1);
    expect(multiPr.length).toBeGreaterThan(0);
    for (const task of multiPr) {
      expect(task.prs.some((pr) => !pr.merged)).toBe(true);
    }
  });
});
