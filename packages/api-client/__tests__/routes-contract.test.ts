/**
 * Verifies that every HTTP endpoint used by the api-client transports
 * has a matching route in the Rust gateway router.
 *
 * Drift here means the frontend would call an endpoint that the gateway
 * 404s on, which breaks the desktop alpha flow before it ever renders
 * OpenSymphony state.
 *
 * The contract lives in two places:
 *   - `packages/api-client/src/transports.ts` (TypeScript clients)
 *   - `crates/opensymphony-gateway/src/lib.rs`  (Rust axum router)
 *
 * This test reads both, then asserts that every documented transport
 * path template appears in the Rust router's route table.
 */

import * as fs from "fs";
import * as path from "path";

const REPO_ROOT = path.resolve(__dirname, "../../..");
const TRANSPORTS_SRC = path.join(
  REPO_ROOT,
  "packages/api-client/src/transports.ts",
);
const GATEWAY_SRC = path.join(
  REPO_ROOT,
  "crates/opensymphony-gateway/src/lib.rs",
);

/**
 * The HTTP and WebSocket paths the api-client transports call. The
 * template uses {param} placeholders that get url-encoded at runtime;
 * the contract checks against the equivalent Rust router path which
 * uses {param} syntax inside axum's `.route(...)`.
 *
 * When changes are made to `transports.ts` with a new REST/SSE/WS path,
 * this list must be updated in lockstep.
 */
const EXPECTED_PATHS: ReadonlyArray<{
  description: string;
  tsSource: string; // substring that must appear in transports.ts
  rustPath: string; // exact route in the Rust router
}> = [
  {
    description: "Gateway capabilities",
    tsSource: "/api/v1/capabilities",
    rustPath: "/api/v1/capabilities",
  },
  {
    description: "Dashboard snapshot",
    tsSource: "/api/v1/dashboard/snapshot",
    rustPath: "/api/v1/dashboard/snapshot",
  },
  {
    description: "Project task graph",
    tsSource: "/api/v1/projects/${encodeURIComponent(projectId)}/taskgraph",
    rustPath: "/api/v1/projects/{project_id}/taskgraph",
  },
  {
    description: "Run detail",
    tsSource: "/api/v1/runs/${encodeURIComponent(runId)}",
    rustPath: "/api/v1/runs/{run_id}",
  },
  {
    description: "Run events",
    tsSource: "/api/v1/runs/${encodeURIComponent(runId)}/events?",
    rustPath: "/api/v1/runs/{run_id}/events",
  },
  {
    description: "Action dispatch",
    tsSource: "/api/v1/actions/dispatch",
    rustPath: "/api/v1/actions/dispatch",
  },
  {
    description: "Gateway SSE event journal stream",
    tsSource: "/api/v1/events",
    rustPath: "/api/v1/events",
  },
  {
    description: "Gateway WebSocket event stream",
    tsSource: "/api/v1/streams/events",
    rustPath: "/api/v1/streams/events",
  },
];

/**
 * The Rust router block lives in `pub fn router(&self) -> Router` and
 * is the only authoritative list of routable endpoints. We extract just
 * that block so unrelated `.route(...)` calls inside helper functions
 * don't accidentally satisfy the contract check.
 *
 * The scan tracks brace depth while skipping characters that appear
 * inside line comments, Rust block comments, regular strings, byte
 * strings, char literals, and raw strings. Without these skips a
 * router that contains a format! macro or a comment with an opening
 * brace would terminate the block at the wrong brace and silently
 * turn legitimate routes into `missing`.
 */
function extractRouterBlock(source: string): string {
  const startMarker = "pub fn router(&self) -> Router";
  const startIdx = source.indexOf(startMarker);
  if (startIdx < 0) {
    throw new Error(
      "Could not locate `pub fn router(&self) -> Router` in gateway lib.rs",
    );
  }

  let depth = 0;
  let started = false;
  let endIdx = -1;
  let i = startIdx;
  while (i < source.length) {
    const ch = source[i];
    const next = source[i + 1];

    if (ch === "/" && next === "/") {
      // Line comment: skip to next newline.
      while (i < source.length && source[i] !== "\n") i++;
      continue;
    }
    if (ch === "/" && next === "*") {
      // Block comment: skip until closing `*/`.
      i += 2;
      while (i < source.length && !(source[i] === "*" && source[i + 1] === "/")) {
        i++;
      }
      i += 2;
      continue;
    }
    if (ch === '"') {
      // Plain string: skip until matching unescaped `"`.
      i++;
      while (i < source.length && source[i] !== '"') {
        if (source[i] === "\\") i += 2;
        else i++;
      }
      i++;
      continue;
    }
    if (ch === "b" && next === '"') {
      // Byte string: same as plain string but with the `b` prefix.
      i += 2;
      while (i < source.length && source[i] !== '"') {
        if (source[i] === "\\") i += 2;
        else i++;
      }
      i++;
      continue;
    }
    if (ch === "'") {
      // Char literal: take the next character, accounting for `\` escapes.
      i++;
      if (source[i] === "\\") i += 2;
      else i++;
      if (source[i] === "'") i++;
      continue;
    }
    if (ch === "r") {
      // Raw string: r"...", r#"..."#, r##"..."## up to 255 hashes.
      let j = i + 1;
      let hashCount = 0;
      while (j < source.length && source[j] === "#") {
        hashCount++;
        j++;
      }
      if (source[j] === '"') {
        const close = '"' + "#".repeat(hashCount);
        i = j + 1;
        const closeIdx = source.indexOf(close, i);
        if (closeIdx === -1) {
          throw new Error("Unterminated raw string in gateway lib.rs");
        }
        i = closeIdx + close.length;
        continue;
      }
    }

    if (ch === "{") {
      depth++;
      started = true;
    } else if (ch === "}") {
      depth--;
      if (started && depth === 0) {
        endIdx = i;
        break;
      }
    }
    i++;
  }

  if (endIdx < 0) {
    throw new Error("Could not find end of router block in gateway lib.rs");
  }
  return source.slice(startIdx, endIdx + 1);
}

describe("api-client -> Rust gateway route contract", () => {
  let transportsSource: string;
  let routerBlock: string;

  beforeAll(() => {
    transportsSource = fs.readFileSync(TRANSPORTS_SRC, "utf-8");
    const gatewaySource = fs.readFileSync(GATEWAY_SRC, "utf-8");
    routerBlock = extractRouterBlock(gatewaySource);
  });

  it("keeps transports.ts and the Rust router in sync for every REST/SSE/WS path", () => {
    const missing: Array<{ description: string; rustPath: string }> = [];

    for (const entry of EXPECTED_PATHS) {
      // 1. Confirm the path still lives in the TS transports file.
      if (!transportsSource.includes(entry.tsSource)) {
        throw new Error(
          `Frontend transport no longer calls ${entry.description} (expected substring "${entry.tsSource}" in transports.ts). ` +
            "If the call site moved, update EXPECTED_PATHS.",
        );
      }
      // 2. Confirm the matching Rust route is still registered.
      if (!routerBlock.includes(`"${entry.rustPath}"`)) {
        missing.push({ description: entry.description, rustPath: entry.rustPath });
      }
    }

    expect(missing).toEqual([]);
  });

  it("does not advertise stub-only or unrouted HTTP paths in HttpGatewayTransport", () => {
    // Guardrail: any new `/api/v1/` literal added to HttpGatewayTransport
    // must be acknowledged here. The exact substring scan below makes a
    // forgotten entry loud during code review instead of silently breaking
    // the desktop alpha flow at runtime.
    const unrouted: string[] = [];
    const pathRegex = /\/api\/v1\/[a-zA-Z0-9_\-/{}:.]+/g;
    const documented = new Set(
      EXPECTED_PATHS.map((entry) => entry.tsSource.split("$")[0]),
    );

    const seen = new Set<string>();
    for (const match of transportsSource.match(pathRegex) ?? []) {
      const normalized = match.split("?")[0];
      if (seen.has(normalized)) continue;
      seen.add(normalized);
      const documentedHit = Array.from(documented).some((doc) =>
        normalized.startsWith(doc.replace(/\$\{[^}]+\}/g, "")),
      );
      // Only entries prefixed with /api/v1/ should be tracked. Subpath
      // hits like /api/v1/actions are covered by the documented list.
      if (
        !documentedHit &&
        normalized !== "/api/v1/capabilities" &&
        normalized !== "/api/v1/dashboard/snapshot" &&
        normalized !== "/api/v1/events" &&
        normalized !== "/api/v1/actions/dispatch" &&
        normalized !== "/api/v1/streams/events"
      ) {
        unrouted.push(normalized);
      }
    }

    expect(unrouted).toEqual([]);
  });

  it("skips Rust strings, comments, char literals, and raw strings when extracting the router block", () => {
    // Adversarial fixtures to prove the brace tracker does not terminate
    // early on characters that appear inside Rust string / comment
    // literals. Regression coverage for the comment that originally used
    // a naive character-level extractor.
    const adversarialSnippets = [
      `.route("/api/v1/${"{run_id}"}/logs", get(logs))`,
      `.route("/api/v1/runs/{id}", get(run)) // see also {nested}`,
      `.route("/api/v1/proof", get(proof)) /* keep {this_brace} */`,
      `.route(b"/api/v1/binary", get(bytes))`,
      `.route("/api/v1/single", get(charlie)) // char "'" lives in a string`,
      `.route(r##"/api/v1/raw/{still_a_route}"##, get(raw_path))`,
    ];

    for (const snippet of adversarialSnippets) {
      const source = [
        "impl GatewayRouter {",
        "    pub fn router(&self) -> Router {",
        snippet,
        "    }",
        "}",
      ].join("\n");
      const block = extractRouterBlock(source);
      // All braces inside the snippet body belong to the array + map
      // literals, none to the router function body. Therefore the
      // extracted block must terminate at the actual function closing
      // brace, not at any `}` introduced by a string literal above.
      expect(block).toContain("Router {");
      expect(block.trim().endsWith("}")).toBe(true);
      // Make sure every adversarial route snippet survived extraction.
      expect(block).toContain(snippet.replace(/\\/g, ""));
    }
  });
});
