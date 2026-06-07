import { HttpGatewayTransport } from "@opensymphony/api-client";
import type { GatewayTransport } from "@opensymphony/api-client";

export interface BrowserTransportAdapter extends GatewayTransport {
  connect(token?: string): Promise<void>;
}

class BrowserTransport implements BrowserTransportAdapter {
  constructor(private readonly inner: GatewayTransport) {}

  get baseUri(): string {
    return this.inner.baseUri;
  }

  health(): ReturnType<GatewayTransport["health"]> {
    return this.inner.health();
  }

  snapshot(): ReturnType<GatewayTransport["snapshot"]> {
    return this.inner.snapshot();
  }

  taskGraph(projectId: string): ReturnType<GatewayTransport["taskGraph"]> {
    return this.inner.taskGraph(projectId);
  }

  runDetail(runId: string): ReturnType<GatewayTransport["runDetail"]> {
    return this.inner.runDetail(runId);
  }

  runEvents(
    runId: string,
    cursor?: Parameters<GatewayTransport["runEvents"]>[1],
  ): ReturnType<GatewayTransport["runEvents"]> {
    return this.inner.runEvents(runId, cursor);
  }

  terminalSnapshot(
    runId: string,
    terminalId: string,
  ): ReturnType<GatewayTransport["terminalSnapshot"]> {
    return this.inner.terminalSnapshot(runId, terminalId);
  }

  events(
    fromCursor?: Parameters<GatewayTransport["events"]>[0],
  ): ReturnType<GatewayTransport["events"]> {
    return this.inner.events(fromCursor);
  }

  terminalFrames(
    runId: string,
  ): ReturnType<GatewayTransport["terminalFrames"]> {
    return this.inner.terminalFrames(runId);
  }

  close(): ReturnType<GatewayTransport["close"]> {
    return this.inner.close();
  }

  async connect(_token?: string): Promise<void> {
    return undefined;
  }
}

export function createWebTransport(baseUri = ""): BrowserTransportAdapter {
  return new BrowserTransport(
    new HttpGatewayTransport({
      baseUri,
      transport: "loopback_http",
    }),
  );
}
