import { computeGraphLayout } from "./index.js";
import type { GraphLayoutOptions, MemoryGraphSnapshot } from "./index.js";

interface LayoutRequest {
  id: number;
  snapshot: MemoryGraphSnapshot;
  options: GraphLayoutOptions;
}

self.onmessage = (event: MessageEvent<LayoutRequest>) => {
  const { id, snapshot, options } = event.data;
  try {
    self.postMessage({ id, result: computeGraphLayout(snapshot, options) });
  } catch (error) {
    self.postMessage({ id, error: error instanceof Error ? error.message : String(error) });
  }
};
