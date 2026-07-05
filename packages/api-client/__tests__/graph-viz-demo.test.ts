/**
 * The graph-viz task-graph demo must keep exercising multi-level dependency
 * routing: deep active chains, blockers fanning to several dependents,
 * diamonds, and long skips — not a single flat level of arrows.
 */

import { graphVizDemoTaskGraph, createGraphVizDemoTransport } from "../src/graph-viz-demo.js";

type Node = (typeof graphVizDemoTaskGraph)["nodes"][number];

function activeIssueNodes(): Node[] {
  return graphVizDemoTaskGraph.nodes.filter(
    (node) => node.kind === "issue" && node.state !== "Done",
  );
}

/** Longest active-blocker chain length ending at each node. */
function dependencyLevels(): Map<string, number> {
  const active = activeIssueNodes();
  const byIdentifier = new Map(active.map((node) => [node.identifier, node]));
  const levels = new Map<string, number>();
  const levelOf = (node: Node, trail: Set<string>): number => {
    const cached = levels.get(node.identifier);
    if (cached !== undefined) return cached;
    if (trail.has(node.identifier)) throw new Error(`dependency cycle through ${node.identifier}`);
    trail.add(node.identifier);
    const blockers = node.blocked_by
      .map((ref) => byIdentifier.get(ref))
      .filter((blocker): blocker is Node => Boolean(blocker));
    const level = blockers.length === 0
      ? 0
      : 1 + Math.max(...blockers.map((blocker) => levelOf(blocker, trail)));
    trail.delete(node.identifier);
    levels.set(node.identifier, level);
    return level;
  };
  for (const node of active) levelOf(node, new Set());
  return levels;
}

describe("graph viz task-graph demo", () => {
  it("keeps at least five simultaneous active dependency levels", () => {
    const levels = dependencyLevels();
    const maxLevel = Math.max(...levels.values());
    expect(maxLevel).toBeGreaterThanOrEqual(5);
    // Every intermediate level is populated, so arrows render at each depth.
    for (let level = 0; level <= maxLevel; level += 1) {
      expect([...levels.values()]).toContain(level);
    }
  });

  it("has blockers fanning to three dependents and re-blocking diamonds", () => {
    const active = activeIssueNodes();
    const dependents = new Map<string, string[]>();
    for (const node of active) {
      for (const blocker of node.blocked_by) {
        dependents.set(blocker, [...(dependents.get(blocker) ?? []), node.identifier]);
      }
    }
    const fanouts = [...dependents.values()].map((list) => list.length);
    // "A blocks B, C, D" and "C blocks E, F, G": at least two three-way fans.
    expect(fanouts.filter((count) => count >= 3).length).toBeGreaterThanOrEqual(2);
    // Diamond: some node is blocked by both a level-0 root and one of that
    // root's own dependents ("B blocks C and D" while A also blocks them).
    const diamond = active.some((node) =>
      node.blocked_by.length >= 2
      && node.blocked_by.some((blockerRef) =>
        active.some((candidate) =>
          candidate.identifier === blockerRef
          && candidate.blocked_by.some((upper) => node.blocked_by.includes(upper)),
        ),
      ));
    expect(diamond).toBe(true);
  });

  it("suppresses arrows from completed blockers by keeping one Done source", () => {
    const done = graphVizDemoTaskGraph.nodes.filter(
      (node) => node.kind === "issue" && node.state === "Done",
    );
    expect(done.length).toBeGreaterThanOrEqual(1);
    const doneIds = new Set(done.map((node) => node.identifier));
    const blockedByDone = graphVizDemoTaskGraph.nodes.filter((node) =>
      node.blocked_by.some((ref) => doneIds.has(ref)),
    );
    expect(blockedByDone.length).toBeGreaterThanOrEqual(1);
  });

  it("serves run details for every running fixture issue", async () => {
    const transport = createGraphVizDemoTransport();
    const running = graphVizDemoTaskGraph.nodes.filter((node) => node.run_id);
    expect(running.length).toBeGreaterThanOrEqual(2);
    for (const node of running) {
      const detail = await transport.runDetail(node.run_id!);
      expect(detail.run_id).toBe(node.run_id);
      expect(detail.status).toBe("running");
    }
  });
});
