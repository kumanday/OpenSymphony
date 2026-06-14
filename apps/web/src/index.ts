import { HttpGatewayTransport } from "@opensymphony/api-client";
import type {
  ActionCapableTransport,
  ActionDispatch,
  ActionReceipt,
  GatewayTransport,
} from "@opensymphony/api-client";

export interface BrowserTransportAdapter extends ActionCapableTransport {
  connect(token?: string): Promise<void>;
}

class BrowserTransport implements BrowserTransportAdapter {
  constructor(private readonly inner: ActionCapableTransport) {}

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

  runTimeline(runId: string): ReturnType<GatewayTransport["runTimeline"]> {
    return this.inner.runTimeline(runId);
  }

  runLogs(
    runId: string,
    cursor?: Parameters<GatewayTransport["runLogs"]>[1],
    limit?: Parameters<GatewayTransport["runLogs"]>[2],
  ): ReturnType<GatewayTransport["runLogs"]> {
    return this.inner.runLogs(runId, cursor, limit);
  }

  terminalSnapshot(
    runId: string,
    terminalId: string,
  ): ReturnType<GatewayTransport["terminalSnapshot"]> {
    return this.inner.terminalSnapshot(runId, terminalId);
  }

  terminalSearch(
    runId: string,
    terminalId: string,
    query: string,
  ): ReturnType<GatewayTransport["terminalSearch"]> {
    return this.inner.terminalSearch(runId, terminalId, query);
  }

  terminalJumpToEvent(
    runId: string,
    terminalId: string,
    eventId: string,
  ): ReturnType<GatewayTransport["terminalJumpToEvent"]> {
    return this.inner.terminalJumpToEvent(runId, terminalId, eventId);
  }

  runFiles(runId: string): ReturnType<GatewayTransport["runFiles"]> {
    return this.inner.runFiles(runId);
  }

  runDiffs(runId: string, filePath?: string): ReturnType<GatewayTransport["runDiffs"]> {
    return this.inner.runDiffs(runId, filePath);
  }

  runApprovals(runId: string): ReturnType<GatewayTransport["runApprovals"]> {
    return this.inner.runApprovals(runId);
  }

  runValidation(runId: string): ReturnType<GatewayTransport["runValidation"]> {
    return this.inner.runValidation(runId);
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

  dispatchAction(action: ActionDispatch): Promise<ActionReceipt> {
    return this.inner.dispatchAction(action);
  }

  cancelRun(runId: string): Promise<ActionReceipt> {
    return this.inner.cancelRun(runId);
  }

  retryRun(runId: string): Promise<ActionReceipt> {
    return this.inner.retryRun(runId);
  }

  resumeRun(runId: string): Promise<ActionReceipt> {
    return this.inner.resumeRun(runId);
  }

  rehydrateRun(runId: string): Promise<ActionReceipt> {
    return this.inner.rehydrateRun(runId);
  }

  commentRun(runId: string, text: string): Promise<ActionReceipt> {
    return this.inner.commentRun(runId, text);
  }

  createFollowup(runId: string, payload: unknown): Promise<ActionReceipt> {
    return this.inner.createFollowup(runId, payload);
  }

  approvalDecision(
    approvalId: string,
    decision: "approved" | "rejected",
    explanation?: string,
  ): Promise<ActionReceipt> {
    return this.inner.approvalDecision(approvalId, decision, explanation);
  }

  openWorkspace(runId: string): Promise<ActionReceipt> {
    return this.inner.openWorkspace(runId);
  }

  debugRun(runId: string): Promise<ActionReceipt> {
    return this.inner.debugRun(runId);
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
