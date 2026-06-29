export function createGraphLayoutWorker(): Worker | null {
  if (typeof Worker === "undefined") return null;
  try {
    return new Worker(new URL("./layout-worker.js", import.meta.url), { type: "module" });
  } catch {
    return null;
  }
}
