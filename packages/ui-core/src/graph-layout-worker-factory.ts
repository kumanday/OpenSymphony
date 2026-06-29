import { createGraphLayoutAdapter, type GraphLayoutAdapter } from "@opensymphony/graph";
import { createGraphLayoutWorker } from "@opensymphony/graph/worker-factory";

export function createBrowserGraphLayoutAdapter(): GraphLayoutAdapter {
  return createGraphLayoutAdapter(createGraphLayoutWorker);
}
