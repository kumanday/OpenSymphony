import { createGraphLayoutAdapter, type GraphLayoutAdapter } from "@opensymphony/graph";

export function createBrowserGraphLayoutAdapter(): GraphLayoutAdapter {
  return createGraphLayoutAdapter(() => {
    if (typeof Worker === "undefined") return null;
    try {
      return new Worker(new URL("../../graph/src/layout-worker.ts", import.meta.url), { type: "module" });
    } catch {
      return null;
    }
  });
}
