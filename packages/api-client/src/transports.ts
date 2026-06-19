import type {
  GatewayEnvelope,
  DashboardSnapshot,
  RunDetail,
  RunEventPage,
  TerminalSnapshot,
  TaskGraphSnapshot,
  GatewayCapabilities,
  ActionDispatch,
  ActionReceipt,
  PageCursor,
  StreamCursor,
  RunTimeline,
  RunLogPage,
  TerminalSearchResult,
  TerminalJumpResult,
  ChangedFileEntry,
  FileDiffPage,
  RunValidationSummary,
  ApprovalRequest,
} from "@opensymphony/gateway-schema";
import { pageCursorFirst } from "@opensymphony/gateway-schema";
import type { GatewayTransport, GatewayTransportConfig, ActionCapableTransport } from "./index.js";
import { stableHash, stableHashJson } from "./util.js";

/**
 * HTTP-based transport adapter using fetch().
 *
 * Supports REST endpoints for snapshots/reads/mutations and SSE
 * for live event streams. Designed to be the baseline contract
 * that all other transport adapters must satisfy.
 */
export class HttpGatewayTransport implements GatewayTransport, ActionCapableTransport {
  readonly baseUri: string;
  private authToken?: string;
  private closed = false;
  private reconnectAttempts = 0;
  private maxReconnectAttempts = 5;
  private reconnectDelayMs = 1000;
  private lastEventTimestamp: number | null = null;
  private streamHealthy = true;
  private readonly streamHealthTimeoutMs = 30_000;
  private abortController: AbortController | null = null;

  constructor(config: GatewayTransportConfig) {
    this.baseUri = config.baseUri.replace(/\/+$/, "");
    this.authToken = config.authToken;
  }

  // -- REST reads --

  async health(): Promise<GatewayCapabilities> {
    const response = await this.fetchJson(`${this.baseUri}/api/v1/capabilities`);
    return response as GatewayCapabilities;
  }

  async snapshot(): Promise<DashboardSnapshot> {
    const response = await this.fetchJson(`${this.baseUri}/api/v1/dashboard/snapshot`);
    return response as DashboardSnapshot;
  }

  async taskGraph(projectId: string): Promise<TaskGraphSnapshot> {
    const response = await this.fetchJson(
      `${this.baseUri}/api/v1/projects/${encodeURIComponent(projectId)}/taskgraph`,
    );
    return response as TaskGraphSnapshot;
  }

  async runDetail(runId: string): Promise<RunDetail> {
    const response = await this.fetchJson(
      `${this.baseUri}/api/v1/runs/${encodeURIComponent(runId)}`,
    );
    return response as RunDetail;
  }

  async runEvents(runId: string, cursor?: PageCursor): Promise<RunEventPage> {
    const params = new URLSearchParams();
    if (cursor?.page_token) params.set("page_token", cursor.page_token);
    params.set("page_size", String(cursor?.page_size ?? 100));
    const response = await this.fetchJson(
      `${this.baseUri}/api/v1/runs/${encodeURIComponent(runId)}/events?${params}`,
    );
    return response as RunEventPage;
  }

  async runTimeline(runId: string): Promise<RunTimeline> {
    const response = await this.fetchJson(
      `${this.baseUri}/api/v1/runs/${encodeURIComponent(runId)}/timeline`,
    );
    return response as RunTimeline;
  }

  async runLogs(
    runId: string,
    cursor?: number,
    limit = 100,
  ): Promise<RunLogPage> {
    const params = new URLSearchParams();
    if (cursor !== undefined) params.set("cursor", String(cursor));
    params.set("limit", String(limit));
    const response = await this.fetchJson(
      `${this.baseUri}/api/v1/runs/${encodeURIComponent(runId)}/logs?${params}`,
    );
    return response as RunLogPage;
  }

  async runFiles(runId: string): Promise<ChangedFileEntry[]> {
    const response = await this.fetchJson(
      `${this.baseUri}/api/v1/runs/${encodeURIComponent(runId)}/files`,
    ) as { files?: ChangedFileEntry[] };
    return response.files ?? [];
  }

  async runDiffs(runId: string, filePath?: string): Promise<FileDiffPage> {
    const params = new URLSearchParams();
    if (filePath) params.set("file_path", filePath);
    const query = params.toString() ? `?${params}` : "";
    const response = await this.fetchJson(
      `${this.baseUri}/api/v1/runs/${encodeURIComponent(runId)}/diffs${query}`,
    );
    return response as FileDiffPage;
  }

  async runApprovals(runId: string): Promise<ApprovalRequest[]> {
    const response = await this.fetchJson(
      `${this.baseUri}/api/v1/runs/${encodeURIComponent(runId)}/approvals`,
    ) as { approvals?: ApprovalRequest[] };
    return response.approvals ?? [];
  }

  async runValidation(runId: string): Promise<RunValidationSummary> {
    const response = await this.fetchJson(
      `${this.baseUri}/api/v1/runs/${encodeURIComponent(runId)}/validation`,
    );
    return response as RunValidationSummary;
  }

  async terminalSnapshot(
    runId: string,
    terminalId: string,
    cursor?: number,
  ): Promise<TerminalSnapshot> {
    const params = new URLSearchParams();
    if (cursor !== undefined) params.set("cursor", String(cursor));
    const query = params.toString() ? `?${params}` : "";
    const response = await this.fetchJson(
      `${this.baseUri}/api/v1/runs/${encodeURIComponent(runId)}/terminal/${encodeURIComponent(terminalId)}${query}`,
    );
    return response as TerminalSnapshot;
  }

  async terminalSearch(
    runId: string,
    terminalId: string,
    query: string,
  ): Promise<TerminalSearchResult> {
    const params = new URLSearchParams({ q: query });
    const response = await this.fetchJson(
      `${this.baseUri}/api/v1/runs/${encodeURIComponent(runId)}/terminal/${encodeURIComponent(terminalId)}/search?${params}`,
    );
    return response as TerminalSearchResult;
  }

  async terminalJumpToEvent(
    runId: string,
    terminalId: string,
    eventId: string,
  ): Promise<TerminalJumpResult> {
    const params = new URLSearchParams({ event_id: eventId });
    const response = await this.fetchJson(
      `${this.baseUri}/api/v1/runs/${encodeURIComponent(runId)}/terminal/${encodeURIComponent(terminalId)}/jump?${params}`,
    );
    return response as TerminalJumpResult;
  }

  // -- Event streams (SSE) --

  async *events(fromCursor?: { sequence: number; partition: string }): AsyncIterable<GatewayEnvelope> {
    const url = new URL(`${this.baseUri}/api/v1/events`);
    if (fromCursor) {
      url.searchParams.set("cursor_sequence", String(fromCursor.sequence));
      url.searchParams.set("cursor_partition", fromCursor.partition);
    }
    yield* this.streamEvents(url);
  }

  async *terminalFrames(runId: string): AsyncIterable<GatewayEnvelope> {
    const url = new URL(
      `${this.baseUri}/api/v1/runs/${encodeURIComponent(runId)}/terminal/stream`,
    );
    yield* this.streamEvents(url);
  }

  /** Shared SSE stream handler with reconnect logic. */
  private async *streamEvents(url: URL): AsyncIterable<GatewayEnvelope> {
    while (!this.closed) {
      let reader: ReadableStreamDefaultReader<Uint8Array> | null = null;
      let shouldReconnect = false;
      try {
        const controller = new AbortController();
        this.abortController = controller;
        const response = await fetch(url.toString(), {
          ...this.buildRequestInit(),
          signal: controller.signal,
        });

        if (!response.ok) {
          console.error(`Stream HTTP error: ${response.status} ${response.statusText}`);
          shouldReconnect = true;
        } else {
          reader = response.body?.getReader() ?? null;
          if (!reader) {
            console.error("Stream response has no readable body");
            shouldReconnect = true;
          } else {
            for await (const envelope of this.parseSSE(reader)) {
              this.lastEventTimestamp = Date.now();
              this.streamHealthy = true;
              this.reconnectAttempts = 0;
              yield envelope;
            }
          }
        }
      } catch (err) {
        if (err instanceof DOMException && err.name === "AbortError") {
          break; // Intentional close.
        }
        console.error("Stream fetch/parse error:", err);
        shouldReconnect = true;
      } finally {
        reader?.releaseLock();
      }

      if (!this.closed && shouldReconnect) {
        this.streamHealthy = false;
        await this.waitForReconnect();
      }
    }
  }

  /** Parse an SSE stream into GatewayEnvelope objects. */
  private async *parseSSE(
    reader: ReadableStreamDefaultReader<Uint8Array>,
  ): AsyncIterable<GatewayEnvelope> {
    const decoder = new TextDecoder();
    let buffer = "";
    let currentEvent = "";
    let currentId = "";
    let currentRetry = 0;
    let currentData = "";

    while (!this.closed) {
      const { done, value } = await reader.read();
      if (done) {
        // Process any remaining buffer content before exiting.
        // First, flush any accumulated currentData.
        if (currentData) {
          try {
            const envelope = JSON.parse(currentData) as GatewayEnvelope;
            yield envelope;
          } catch (err) {
            console.error("SSE parse error: malformed JSON event data (trailing buffer)", err);
          }
        }
        // Also process any remaining buffer that might contain a partial event.
        if (buffer.trim()) {
          // Treat remaining buffer as potential data if it doesn't start with a field prefix.
          const remainingLines = buffer.trim().split("\n");
          let pendingData = "";
          for (const line of remainingLines) {
            if (line.startsWith("data: ")) {
              pendingData += (pendingData ? "\n" : "") + line.slice(6);
            } else if (line === "") {
              // Empty line marks event boundary.
              if (pendingData) {
                try {
                  const envelope = JSON.parse(pendingData) as GatewayEnvelope;
                  yield envelope;
                } catch (err) {
                  console.error("SSE parse error: malformed JSON event data (buffer flush)", err);
                }
                pendingData = "";
              }
            }
          }
          // Flush any remaining pending data.
          if (pendingData) {
            try {
              const envelope = JSON.parse(pendingData) as GatewayEnvelope;
              yield envelope;
            } catch (err) {
              console.error("SSE parse error: malformed JSON event data (final buffer)", err);
            }
          }
        }
        break;
      }

      buffer += decoder.decode(value, { stream: true });
      const lines = buffer.split("\n");
      buffer = lines.pop() ?? "";

      for (const line of lines) {
        // Empty line = end of event block.
        if (line === "") {
          if (currentData) {
            try {
              const envelope = JSON.parse(currentData) as GatewayEnvelope;
              yield envelope;
            } catch (err) {
              console.error("SSE parse error: malformed JSON event data", err);
            }
            currentEvent = "";
            currentId = "";
            currentRetry = 0;
            currentData = "";
          }
          continue;
        }

        if (line.startsWith("event: ")) {
          currentEvent = line.slice(7);
        } else if (line.startsWith("id: ")) {
          currentId = line.slice(4);
        } else if (line.startsWith("retry: ")) {
          currentRetry = parseInt(line.slice(7), 10) || 0;
        } else if (line.startsWith("data: ")) {
          // Multi-line data: append with newline.
          if (currentData) currentData += "\n";
          currentData += line.slice(6);
        } else if (line.startsWith(":")) {
          // SSE comment line, ignore.
        }
        // Per SSE spec, unrecognized field names are discarded.
      }

      if (currentRetry > 0) {
        this.reconnectDelayMs = currentRetry;
      }
    }
  }

  // -- Action mutations --

  async dispatchAction(action: ActionDispatch): Promise<ActionReceipt> {
    const response = await this.fetchJson(
      `${this.baseUri}/api/v1/actions/dispatch`,
      {
        method: "POST",
        body: JSON.stringify(action),
      },
    );
    return response as ActionReceipt;
  }

  async cancelRun(runId: string): Promise<ActionReceipt> {
    return this.dispatchAction({
      schema_version: { major: 1, minor: 0, patch: 0 },
      correlation_id: `cancel-${runId}-${crypto.randomUUID()}`,
      action_kind: "cancel",
      target_entity: { entity_kind: "run", entity_id: runId },
      idempotency_key: `cancel-${runId}`,
    });
  }

  async retryRun(runId: string): Promise<ActionReceipt> {
    return this.dispatchAction({
      schema_version: { major: 1, minor: 0, patch: 0 },
      correlation_id: `retry-${runId}-${crypto.randomUUID()}`,
      action_kind: "retry",
      target_entity: { entity_kind: "run", entity_id: runId },
      idempotency_key: `retry-${runId}`,
    });
  }

  async resumeRun(runId: string): Promise<ActionReceipt> {
    return this.dispatchAction({
      schema_version: { major: 1, minor: 0, patch: 0 },
      correlation_id: `resume-${runId}-${crypto.randomUUID()}`,
      action_kind: "resume",
      target_entity: { entity_kind: "run", entity_id: runId },
      idempotency_key: `resume-${runId}`,
    });
  }

  async rehydrateRun(runId: string): Promise<ActionReceipt> {
    return this.dispatchAction({
      schema_version: { major: 1, minor: 0, patch: 0 },
      correlation_id: `rehydrate-${runId}-${crypto.randomUUID()}`,
      action_kind: "rehydrate",
      target_entity: { entity_kind: "run", entity_id: runId },
      idempotency_key: `rehydrate-${runId}`,
    });
  }

  async commentRun(runId: string, text: string): Promise<ActionReceipt> {
    return this.dispatchAction({
      schema_version: { major: 1, minor: 0, patch: 0 },
      correlation_id: `comment-${runId}-${crypto.randomUUID()}`,
      action_kind: "comment",
      target_entity: { entity_kind: "run", entity_id: runId },
      payload: { text },
      idempotency_key: `comment-${runId}-${await stableHash(text)}`,
    });
  }

  async createFollowup(runId: string, payload: unknown): Promise<ActionReceipt> {
    return this.dispatchAction({
      schema_version: { major: 1, minor: 0, patch: 0 },
      correlation_id: `followup-${runId}-${crypto.randomUUID()}`,
      action_kind: "create_followup",
      target_entity: { entity_kind: "run", entity_id: runId },
      payload,
      idempotency_key: `followup-${runId}-${await stableHashJson(payload)}`,
    });
  }

  async approvalDecision(
    approvalId: string,
    decision: "approved" | "rejected",
    explanation?: string,
  ): Promise<ActionReceipt> {
    return this.dispatchAction({
      schema_version: { major: 1, minor: 0, patch: 0 },
      correlation_id: `approval-${approvalId}-${crypto.randomUUID()}`,
      action_kind: "approval_decision",
      target_entity: { entity_kind: "approval", entity_id: approvalId },
      payload: { decision, explanation },
      idempotency_key: `approval-${approvalId}-${decision}`,
    });
  }

  async openWorkspace(runId: string): Promise<ActionReceipt> {
    return this.dispatchAction({
      schema_version: { major: 1, minor: 0, patch: 0 },
      correlation_id: `workspace-${runId}-${crypto.randomUUID()}`,
      action_kind: "open_workspace",
      target_entity: { entity_kind: "run", entity_id: runId },
      idempotency_key: `workspace-${runId}`,
    });
  }

  async debugRun(runId: string): Promise<ActionReceipt> {
    return this.dispatchAction({
      schema_version: { major: 1, minor: 0, patch: 0 },
      correlation_id: `debug-${runId}-${crypto.randomUUID()}`,
      action_kind: "debug",
      target_entity: { entity_kind: "run", entity_id: runId },
      idempotency_key: `debug-${runId}`,
    });
  }

  // -- Lifecycle --

  async close(): Promise<void> {
    this.closed = true;
    this.abortController?.abort();
  }

  // -- Stream health diagnostics --

  /** Whether the stream has received events recently. */
  isStreamHealthy(): boolean {
    if (this.lastEventTimestamp === null) return true;
    return Date.now() - this.lastEventTimestamp < this.streamHealthTimeoutMs;
  }

  /** Reconnect attempt count since last successful event. */
  getReconnectAttempts(): number {
    return this.reconnectAttempts;
  }

  // -- Private helpers --

  private buildRequestInit(): RequestInit {
    const init: RequestInit = {
      headers: {
        Accept: "text/event-stream",
      },
    };
    if (this.authToken) {
      init.headers = {
        ...init.headers,
        Authorization: `Bearer ${this.authToken}`,
      };
    }
    return init;
  }

  private async fetchJson(url: string, init?: RequestInit): Promise<unknown> {
    const method = init?.method ?? "GET";
    const headers: Record<string, string> = {
      ...(init?.headers as Record<string, string> ?? {}),
    };

    // Only set Content-Type for requests with a body.
    if (method !== "GET" && method !== "HEAD") {
      headers["Content-Type"] = "application/json";
    }

    const requestInit: RequestInit = { ...init, headers };
    if (this.authToken) {
      requestInit.headers = {
        ...requestInit.headers,
        Authorization: `Bearer ${this.authToken}`,
      };
    }

    const response = await fetch(url, requestInit);

    if (!response.ok) {
      const body = await response.text().catch(() => "");
      throw new Error(
        `HTTP ${response.status} ${response.statusText}: ${body}`,
      );
    }

    return response.json();
  }

  private async waitForReconnect(): Promise<void> {
    this.reconnectAttempts++;
    if (this.reconnectAttempts >= this.maxReconnectAttempts) {
      throw new Error(
        `Max reconnect attempts (${this.maxReconnectAttempts}) reached`,
      );
    }
    const delay = this.reconnectDelayMs * Math.pow(2, this.reconnectAttempts - 1);
    await new Promise((resolve) => setTimeout(resolve, delay));
  }
}

export class WebSocketTransport implements GatewayTransport {
  readonly baseUri: string;
  private readonly authToken?: string;
  private ws?: WebSocket;
  private eventSubscribers: Set<(envelope: GatewayEnvelope) => void> = new Set();
  private terminalSubscribers: Map<string, Set<(envelope: GatewayEnvelope) => void>> = new Map();
  private pendingGeneratorCancellers: Set<() => void> = new Set();
  private reconnectDelayMs = 1000;
  private maxReconnectDelayMs = 30000;
  private isReconnecting = false;
  private isClosed = false;
  private connecting?: Promise<void>;
  private lastEventCursor?: { sequence: number; partition: string };

  constructor(config: GatewayTransportConfig) {
    this.baseUri = config.baseUri.replace(/\/+$/, "");
    this.authToken = config.authToken;
  }

  private wsUrl(path: string): string {
    const uri = this.baseUri.replace(/^http/, "ws");
    return `${uri}${path}`;
  }

  private headers(): Record<string, string> {
    const headers: Record<string, string> = {
      "Content-Type": "application/json",
      Accept: "application/json",
    };
    if (this.authToken) {
      headers["Authorization"] = `Bearer ${this.authToken}`;
    }
    return headers;
  }

  private async get<T>(path: string): Promise<T> {
    const url = `${this.baseUri}${path}`;
    const response = await fetch(url, {
      method: "GET",
      headers: this.headers(),
    });
    if (!response.ok) {
      throw new Error(
        `HTTP ${response.status} from ${url}: ${response.statusText}`,
      );
    }
    return (await response.json()) as T;
  }

  async health(): Promise<GatewayCapabilities> {
    return this.get<GatewayCapabilities>("/api/v1/capabilities");
  }

  async snapshot(): Promise<DashboardSnapshot> {
    return this.get<DashboardSnapshot>("/api/v1/dashboard/snapshot");
  }

  async taskGraph(projectId: string): Promise<TaskGraphSnapshot> {
    return this.get<TaskGraphSnapshot>(
      `/api/v1/projects/${encodeURIComponent(projectId)}/taskgraph`,
    );
  }

  async runDetail(runId: string): Promise<RunDetail> {
    return this.get<RunDetail>(`/api/v1/runs/${encodeURIComponent(runId)}`);
  }

  async runEvents(
    runId: string,
    cursor?: PageCursor,
  ): Promise<RunEventPage> {
    const pageCursor = cursor ?? pageCursorFirst(100);
    const params = new URLSearchParams();
    if (pageCursor.page_token) {
      params.set("page_token", pageCursor.page_token);
    }
    params.set("page_size", String(pageCursor.page_size));
    return this.get<RunEventPage>(
      `/api/v1/runs/${encodeURIComponent(runId)}/events?${params.toString()}`,
    );
  }

  async runTimeline(runId: string): Promise<RunTimeline> {
    return this.get<RunTimeline>(
      `/api/v1/runs/${encodeURIComponent(runId)}/timeline`,
    );
  }

  async runLogs(
    runId: string,
    cursor?: number,
    limit = 100,
  ): Promise<RunLogPage> {
    const params = new URLSearchParams();
    if (cursor !== undefined) params.set("cursor", String(cursor));
    params.set("limit", String(limit));
    return this.get<RunLogPage>(
      `/api/v1/runs/${encodeURIComponent(runId)}/logs?${params.toString()}`,
    );
  }

  async terminalSnapshot(
    runId: string,
    terminalId: string,
    cursor?: number,
  ): Promise<TerminalSnapshot> {
    const params = new URLSearchParams();
    if (cursor !== undefined) params.set("cursor", String(cursor));
    const query = params.toString() ? `?${params.toString()}` : "";
    return this.get<TerminalSnapshot>(
      `/api/v1/runs/${encodeURIComponent(runId)}/terminal/${encodeURIComponent(terminalId)}${query}`,
    );
  }

  async terminalSearch(
    runId: string,
    terminalId: string,
    query: string,
  ): Promise<TerminalSearchResult> {
    const params = new URLSearchParams({ q: query });
    return this.get<TerminalSearchResult>(
      `/api/v1/runs/${encodeURIComponent(runId)}/terminal/${encodeURIComponent(terminalId)}/search?${params.toString()}`,
    );
  }

  async terminalJumpToEvent(
    runId: string,
    terminalId: string,
    eventId: string,
  ): Promise<TerminalJumpResult> {
    const params = new URLSearchParams({ event_id: eventId });
    return this.get<TerminalJumpResult>(
      `/api/v1/runs/${encodeURIComponent(runId)}/terminal/${encodeURIComponent(terminalId)}/jump?${params.toString()}`,
    );
  }

  async runFiles(runId: string): Promise<ChangedFileEntry[]> {
    const response = await this.get<{ files?: ChangedFileEntry[] }>(
      `/api/v1/runs/${encodeURIComponent(runId)}/files`,
    );
    return response.files ?? [];
  }

  async runDiffs(runId: string, filePath?: string): Promise<FileDiffPage> {
    const params = new URLSearchParams();
    if (filePath) params.set("file_path", filePath);
    const query = params.toString() ? `?${params.toString()}` : "";
    return this.get<FileDiffPage>(
      `/api/v1/runs/${encodeURIComponent(runId)}/diffs${query}`,
    );
  }

  async runApprovals(runId: string): Promise<ApprovalRequest[]> {
    const response = await this.get<{ approvals?: ApprovalRequest[] }>(
      `/api/v1/runs/${encodeURIComponent(runId)}/approvals`,
    );
    return response.approvals ?? [];
  }

  async runValidation(runId: string): Promise<RunValidationSummary> {
    return this.get<RunValidationSummary>(
      `/api/v1/runs/${encodeURIComponent(runId)}/validation`,
    );
  }

  private async ensureConnected(
    fromCursor?: { sequence: number; partition: string },
  ): Promise<void> {
    if (this.isClosed) {
      return;
    }
    // If already connected but the cursor has changed, we need to reconnect
    // to establish the new subscription point on the server.
    if (this.ws && this.ws.readyState === WebSocket.OPEN) {
      if (
        fromCursor &&
        (!this.lastEventCursor ||
          this.lastEventCursor.sequence !== fromCursor.sequence ||
          this.lastEventCursor.partition !== fromCursor.partition)
      ) {
        // Close the existing socket so reconnect uses the new cursor
        this.ws.close();
        this.ws = undefined;
      } else {
        return;
      }
    }
    this.connecting ??= this.connectWebSocket(fromCursor).finally(() => {
      this.connecting = undefined;
    });
    await this.connecting;
  }

  private async connectWebSocket(
    fromCursor?: { sequence: number; partition: string },
  ): Promise<void> {
    if (this.ws) {
      this.ws.onclose = null;
      this.ws.onerror = null;
      this.ws.onmessage = null;
      this.ws.close();
    }

    const WS_CONNECT_TIMEOUT_MS = 10_000;
    return new Promise((resolve, reject) => {
      let url = this.wsUrl("/api/v1/streams/events");
      if (fromCursor) {
        const urlObj = new URL(url);
        urlObj.searchParams.set("cursor_sequence", String(fromCursor.sequence));
        urlObj.searchParams.set("cursor_partition", fromCursor.partition);
        url = urlObj.toString();
      }
      const ws = new WebSocket(url);
      this.ws = ws;
      let hasOpened = false;

      const timeoutId = setTimeout(() => {
        if (ws.readyState === WebSocket.CONNECTING) {
          ws.close();
          reject(new Error(`WebSocket connection timed out after ${WS_CONNECT_TIMEOUT_MS}ms`));
        }
      }, WS_CONNECT_TIMEOUT_MS);

      ws.onopen = () => {
        clearTimeout(timeoutId);
        hasOpened = true;
        this.reconnectDelayMs = 1000; // Reset reconnect delay after successful connection
        // Record the cursor that was used for this connection
        if (fromCursor) {
          this.lastEventCursor = { ...fromCursor };
        } else {
          this.lastEventCursor = undefined;
        }
        // Send auth if needed
        if (this.authToken) {
          ws.send(JSON.stringify({ type: "auth", token: this.authToken }));
        }
        resolve();
      };

      ws.onerror = (error) => {
        clearTimeout(timeoutId);
        // Only reject if we haven't resolved yet
        if (!hasOpened) {
          reject(error instanceof Error ? error : new Error('WebSocket connection error'));
        }
      };

      ws.onclose = (event) => {
        clearTimeout(timeoutId);
        if (this.ws === ws) {
          this.ws = undefined;
        }
        if (this.isClosed) {
          return;
        }
        if (!hasOpened) {
          reject(new Error(`WebSocket connection closed during handshake (code: ${event.code}, reason: ${event.reason || 'none'})`));
        } else {
          this.scheduleReconnect();
        }
      };

      ws.onmessage = (event) => {
        this.handleMessage(event.data);
      };
    });
  }

  private dispatch(envelope: GatewayEnvelope): void {
    this.eventSubscribers.forEach((cb) => cb(envelope));
    if (envelope.entity_ref.kind !== "terminal_session") {
      return;
    }

    const match = envelope.cursor.partition.match(/^[^:]+:(.+)$/);
    const runId = match?.[1] ?? envelope.cursor.partition;
    if (runId) {
      this.terminalSubscribers.get(runId)?.forEach((cb) => cb(envelope));
    }
  }

  private terminalSubscriberSet(
    runId: string,
  ): Set<(envelope: GatewayEnvelope) => void> {
    let subscribers = this.terminalSubscribers.get(runId);
    if (!subscribers) {
      subscribers = new Set();
      this.terminalSubscribers.set(runId, subscribers);
    }
    return subscribers;
  }

  private handleMessage(data: string): void {
    // Gateway uses prefixed frames: "__event__ {...}" or "__error__ {...}"
    if (data.startsWith("__event__ ")) {
      try {
        const payload = data.slice(10);
        const envelope = JSON.parse(payload) as GatewayEnvelope;
        this.dispatch(envelope);
      } catch {
        // Skip malformed messages
      }
    } else if (data.startsWith("__error__ ")) {
      // Handle stream error - could trigger reconnect
      try {
        const payload = data.slice(10);
        const error = JSON.parse(payload);
        if (error.recoverable) {
          this.scheduleReconnect();
        }
      } catch {
        // Skip malformed errors
      }
    } else {
      // Try parsing as direct JSON envelope (legacy format)
      try {
        const envelope = JSON.parse(data) as GatewayEnvelope;
        this.dispatch(envelope);
      } catch {
        // Skip unknown message formats
      }
    }
  }

  private scheduleReconnect(): void {
    if (this.isClosed || this.isReconnecting) return;
    this.isReconnecting = true;

    const delay = Math.min(
      this.reconnectDelayMs * 2,
      this.maxReconnectDelayMs,
    );
    this.reconnectDelayMs = delay;

    setTimeout(() => {
      this.isReconnecting = false;
      if (this.isClosed) {
        return;
      }
      this.ensureConnected().catch(() => {
        // Reconnect will be scheduled again on close
      });
    }, delay);
  }

  async *events(fromCursor?: { sequence: number; partition: string }): AsyncIterable<GatewayEnvelope> {
    await this.ensureConnected(fromCursor);

    // Create a promise-based queue for this subscriber
    const queue: GatewayEnvelope[] = [];
    let resolveNext: ((value: IteratorResult<GatewayEnvelope>) => void) | null = null;

    const subscriber = (envelope: GatewayEnvelope) => {
      queue.push(envelope);
      if (resolveNext) {
        resolveNext({ value: envelope, done: false });
        resolveNext = null;
      }
    };

    // Track this generator's resolve function for cleanup on close
    const cancelGenerator = () => {
      if (resolveNext) {
        resolveNext({ value: {} as GatewayEnvelope, done: true });
        resolveNext = null;
      }
    };
    this.pendingGeneratorCancellers.add(cancelGenerator);

    this.eventSubscribers.add(subscriber);

    try {
      while (!this.isClosed) {
        if (queue.length > 0) {
          yield queue.shift()!;
        } else {
          await new Promise<IteratorResult<GatewayEnvelope>>((resolve) => {
            resolveNext = resolve;
          });
        }
      }
    } finally {
      this.eventSubscribers.delete(subscriber);
      this.pendingGeneratorCancellers.delete(cancelGenerator);
    }
  }

  async *terminalFrames(runId: string): AsyncIterable<GatewayEnvelope> {
    await this.ensureConnected();

    const queue: GatewayEnvelope[] = [];
    let resolveNext: ((value: IteratorResult<GatewayEnvelope>) => void) | null = null;

    const subscriber = (envelope: GatewayEnvelope) => {
      if (envelope.entity_ref.kind === "terminal_session") {
        queue.push(envelope);
        if (resolveNext) {
          resolveNext({ value: envelope, done: false });
          resolveNext = null;
        }
      }
    };

    // Track this generator's resolve function for cleanup on close
    const cancelGenerator = () => {
      if (resolveNext) {
        resolveNext({ value: {} as GatewayEnvelope, done: true });
        resolveNext = null;
      }
    };
    this.pendingGeneratorCancellers.add(cancelGenerator);

    this.terminalSubscriberSet(runId).add(subscriber);

    try {
      while (!this.isClosed) {
        if (queue.length > 0) {
          yield queue.shift()!;
        } else {
          await new Promise<IteratorResult<GatewayEnvelope>>((resolve) => {
            resolveNext = resolve;
          });
        }
      }
    } finally {
      const subs = this.terminalSubscribers.get(runId);
      if (subs) {
        subs.delete(subscriber);
        if (subs.size === 0) {
          this.terminalSubscribers.delete(runId);
        }
      }
      this.pendingGeneratorCancellers.delete(cancelGenerator);
    }
  }

  async close(): Promise<void> {
    this.isClosed = true;
    
    // Resolve all pending generator promises to prevent memory leaks and hangs
    for (const cancel of this.pendingGeneratorCancellers) {
      cancel();
    }
    this.pendingGeneratorCancellers.clear();
    
    if (this.ws) {
      this.ws.onclose = null;
      this.ws.close();
      this.ws = undefined;
    }
    this.connecting = undefined;
    this.eventSubscribers.clear();
    this.terminalSubscribers.clear();
  }
}

/**
 * Tauri channel transport adapter for desktop local mode.
 *
 * Uses Tauri's invoke/channel system for high-performance local communication
 * between the Rust backend and webview frontend. This transport is optimized
 * for local gateway connections where the orchestrator runs on the same machine.
 *
 * In the preferred transport order:
 * 1. In-process Rust channels (when embedded) - not available in webview
 * 2. Native IPC (Unix sockets/named pipes) - via loopback fallback
 * 3. Tauri channels (this transport) - high-volume frames to webview
 * 4. Loopback HTTP/WebSocket - compatibility baseline
 */

/**
 * Tauri IPC Channel shape compatible with @tauri-apps/api/core.Channel.
 *
 * In Tauri v2, the frontend creates a Channel via `new Channel<T>(onMessage)`,
 * passes it as `tx` to `invoke(cmd, { tx })`, and the backend receives it as
 * `tauri::ipc::Channel<T>`.
 */
export interface TauriChannel<T> {
  onmessage: ((data: T) => void) | null;
  close(): void;
}

/**
 * Tauri runtime API shape available on `globalThis.__TAURI__`.
 * Provides the `invoke` function and the `Channel` constructor.
 */
export interface TauriRuntime {
  invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T>;
  core: {
    Channel<T>(onMessage: (data: T) => void): TauriChannel<T>;
  };
}

export class TauriChannelTransport implements GatewayTransport {
  readonly baseUri: string;
  private eventChannels = new Set<TauriChannel<GatewayEnvelope>>();
  private terminalChannels: Map<string, Set<TauriChannel<GatewayEnvelope>>> = new Map();
  private isClosed = false;
  private readonly pendingGeneratorCancellers = new Set<() => void>();
  private readonly pendingGeneratorCleanups = new Set<() => Promise<void>>();

  constructor(config: GatewayTransportConfig) {
    this.baseUri = config.baseUri.replace(/\/+$/, "");
  }

  private tauri(): TauriRuntime {
    const tauri = (globalThis as Record<string, unknown>).__TAURI__ as
      | TauriRuntime
      | undefined;

    if (!tauri?.invoke || !tauri?.core?.Channel) {
      throw new Error(
        "TauriChannelTransport requires Tauri v2 runtime with invoke and Channel",
      );
    }

    return tauri;
  }

  private async invoke<T>(command: string, args?: Record<string, unknown>): Promise<T> {
    return this.tauri().invoke(command, args ?? {});
  }

  async health(): Promise<GatewayCapabilities> {
    return this.invoke<GatewayCapabilities>("gateway_capabilities", {});
  }

  async snapshot(): Promise<DashboardSnapshot> {
    return this.invoke<DashboardSnapshot>("dashboard_snapshot", {});
  }

  async taskGraph(projectId: string): Promise<TaskGraphSnapshot> {
    return this.invoke<TaskGraphSnapshot>("task_graph", { project_id: projectId });
  }

  async runDetail(runId: string): Promise<RunDetail> {
    return this.invoke<RunDetail>("run_detail", { run_id: runId });
  }

  async runEvents(runId: string, cursor?: PageCursor): Promise<RunEventPage> {
    return this.invoke<RunEventPage>("run_events", {
      run_id: runId,
      page_token: cursor?.page_token ?? null,
      page_size: cursor?.page_size ?? null,
    });
  }

  async runTimeline(runId: string): Promise<RunTimeline> {
    return this.invoke<RunTimeline>("run_timeline", { run_id: runId });
  }

  async runLogs(
    runId: string,
    _cursor?: number,
    _limit = 100,
  ): Promise<RunLogPage> {
    return this.invoke<RunLogPage>("run_logs", { run_id: runId });
  }

  async terminalSnapshot(
    runId: string,
    terminalId: string,
    cursor?: number,
  ): Promise<TerminalSnapshot> {
    return this.invoke<TerminalSnapshot>("terminal_snapshot", {
      run_id: runId,
      terminal_id: terminalId,
      cursor,
    });
  }

  async terminalSearch(
    runId: string,
    terminalId: string,
    query: string,
  ): Promise<TerminalSearchResult> {
    return this.invoke<TerminalSearchResult>("terminal_search", {
      run_id: runId,
      terminal_id: terminalId,
      q: query,
    });
  }

  async terminalJumpToEvent(
    runId: string,
    terminalId: string,
    eventId: string,
  ): Promise<TerminalJumpResult> {
    return this.invoke<TerminalJumpResult>("terminal_jump_to_event", {
      run_id: runId,
      terminal_id: terminalId,
      event_id: eventId,
    });
  }

  async runFiles(runId: string): Promise<ChangedFileEntry[]> {
    return this.invoke<{ files: ChangedFileEntry[] }>("run_files", {
      run_id: runId,
    }).then((r) => r.files ?? []);
  }

  async runDiffs(runId: string, filePath?: string): Promise<FileDiffPage> {
    return this.invoke<FileDiffPage>("run_diffs", {
      run_id: runId,
      file_path: filePath,
    });
  }

  async runApprovals(runId: string): Promise<ApprovalRequest[]> {
    return this.invoke<{ approvals: ApprovalRequest[] }>("run_approvals", {
      run_id: runId,
    }).then((r) => r.approvals ?? []);
  }

  async runValidation(runId: string): Promise<RunValidationSummary> {
    return this.invoke<RunValidationSummary>("run_validation", { run_id: runId });
  }

  async *events(fromCursor?: { sequence: number; partition: string }): AsyncIterable<GatewayEnvelope> {
    const queue: GatewayEnvelope[] = [];
    let resolveNext: ((value: IteratorResult<GatewayEnvelope>) => void) | null = null;

    const channel = this.tauri().core.Channel<GatewayEnvelope>(
      (envelope: GatewayEnvelope) => {
        queue.push(envelope);
        if (resolveNext) {
          resolveNext({ value: envelope, done: false });
          resolveNext = null;
        }
      },
    );

    const cancelGenerator = () => {
      if (resolveNext) {
        resolveNext({ value: {} as GatewayEnvelope, done: true });
        resolveNext = null;
      }
    };
    this.pendingGeneratorCancellers.add(cancelGenerator);

    let cleanedUp = false;
    const cleanup = async () => {
      if (cleanedUp) {
        return;
      }
      cleanedUp = true;
      this.pendingGeneratorCancellers.delete(cancelGenerator);
      this.pendingGeneratorCleanups.delete(cleanup);
      this.eventChannels.delete(channel);
      channel.close?.();
      await this.invoke("unsubscribe_events", {}).catch(() => undefined);
    };
    this.pendingGeneratorCleanups.add(cleanup);

    const args: Record<string, unknown> = { tx: channel };
    if (fromCursor) {
      args.cursor = fromCursor.sequence;
      args.partition = fromCursor.partition;
    }
    // Pass the frontend-created channel as `tx` to the Rust backend.
    // The backend receives this as `tauri::ipc::Channel<GatewayEnvelope>`.
    await this.invoke("subscribe_events", args);
    this.eventChannels.add(channel);

    try {
      while (!this.isClosed) {
        if (queue.length > 0) {
          yield queue.shift()!;
        } else {
          await new Promise<IteratorResult<GatewayEnvelope>>((resolve) => {
            resolveNext = resolve;
          });
        }
      }
    } finally {
      await cleanup();
    }
  }

  async *terminalFrames(runId: string): AsyncIterable<GatewayEnvelope> {
    const queue: GatewayEnvelope[] = [];
    let resolveNext: ((value: IteratorResult<GatewayEnvelope>) => void) | null = null;

    const channel = this.tauri().core.Channel<GatewayEnvelope>(
      (envelope: GatewayEnvelope) => {
        if (envelope.entity_ref.kind === "terminal_session") {
          queue.push(envelope);
          if (resolveNext) {
            resolveNext({ value: envelope, done: false });
            resolveNext = null;
          }
        }
      },
    );

    const cancelGenerator = () => {
      if (resolveNext) {
        resolveNext({ value: {} as GatewayEnvelope, done: true });
        resolveNext = null;
      }
    };
    this.pendingGeneratorCancellers.add(cancelGenerator);

    let cleanedUp = false;
    const cleanup = async () => {
      if (cleanedUp) {
        return;
      }
      cleanedUp = true;
      this.pendingGeneratorCancellers.delete(cancelGenerator);
      this.pendingGeneratorCleanups.delete(cleanup);
      const channels = this.terminalChannels.get(runId);
      channels?.delete(channel);
      if (channels?.size === 0) {
        this.terminalChannels.delete(runId);
      }
      channel.close?.();
      await this.invoke("unsubscribe_terminal", { run_id: runId }).catch(() => undefined);
    };
    this.pendingGeneratorCleanups.add(cleanup);

    // Pass the frontend-created channel as `tx` to the Rust backend.
    await this.invoke("subscribe_terminal", { run_id: runId, tx: channel });

    const channels = this.terminalChannels.get(runId);
    if (channels) {
      channels.add(channel);
    } else {
      this.terminalChannels.set(runId, new Set([channel]));
    }

    try {
      while (!this.isClosed) {
        if (queue.length > 0) {
          yield queue.shift()!;
        } else {
          await new Promise<IteratorResult<GatewayEnvelope>>((resolve) => {
            resolveNext = resolve;
          });
        }
      }
    } finally {
      await cleanup();
    }
  }

  async close(): Promise<void> {
    this.isClosed = true;

    // Resolve all pending generator promises to prevent hangs
    for (const cancel of this.pendingGeneratorCancellers) {
      cancel();
    }

    const cleanups = Array.from(this.pendingGeneratorCleanups);
    await Promise.allSettled(cleanups.map((cleanup) => cleanup()));
    this.pendingGeneratorCancellers.clear();
    this.pendingGeneratorCleanups.clear();
    this.eventChannels.clear();
    this.terminalChannels.clear();
  }
}

/**
 * Transport factory that selects the best available transport profile
 * based on the gateway capabilities and connection configuration.
 *
 * Preferred transport order for desktop local mode:
 * 1. In-process Rust channels (embedded host) - lowest latency
 * 2. Native local IPC (separate local process) - Unix sockets/named pipes
 * 3. Tauri channels (Rust backend to webview) - high-volume frames
 * 4. Loopback HTTP/WebSocket - compatibility baseline
 */

export class TransportFactory {
  /**
   * Create a transport based on the recommended profile and available capabilities.
   * Falls back to loopback HTTP if the preferred transport is unavailable.
   */
  static async create(
    config: GatewayTransportConfig,
    capabilities?: GatewayCapabilities,
  ): Promise<GatewayTransport> {
    const profile = config.transport ?? "loopback_http";

    // If we have capabilities, verify the transport is supported
    if (capabilities) {
      const transportCap = capabilities.transports.find(
        (t) => t.transport === profile,
      );
      if (!transportCap) {
        // Fall back to loopback HTTP
        return new HttpGatewayTransport(config);
      }
    }

    switch (profile) {
      case "in_process_channel":
      case "native_ipc":
      case "tauri_channel":
        // These require Tauri runtime context
        if (typeof (globalThis as Record<string, unknown>).__TAURI__ !== "undefined") {
          return new TauriChannelTransport(config);
        }
        // Fall through to HTTP if Tauri not available
        return new HttpGatewayTransport(config);

      case "loopback_http":
      case "sse":
        return new HttpGatewayTransport(config);

      case "loopback_websocket":
      case "websocket":
      case "json_rpc_over_websocket":
        // Check if WebSocket is available
        if (typeof WebSocket !== "undefined") {
          return new WebSocketTransport(config);
        }
        // Fall back to HTTP
        return new HttpGatewayTransport(config);

      default:
        return new HttpGatewayTransport(config);
    }
  }

  /**
   * Determine the best transport profile for the current environment.
   * Returns profiles in order of preference.
   */
  static getPreferredProfiles(): Array<{
    profile: string;
    available: boolean;
    description: string;
  }> {
    const isTauri =
      typeof (globalThis as Record<string, unknown>).__TAURI__ !== "undefined";
    const hasWebSocket = typeof WebSocket !== "undefined";

    return [
      {
        profile: isTauri ? "tauri_channel" : "in_process_channel",
        available: isTauri,
        description: isTauri
          ? "Tauri channels (Rust backend to webview)"
          : "In-process Rust channels (embedded host)",
      },
      {
        profile: "native_ipc",
        available: typeof process !== "undefined",
        description: "Native local IPC (Unix sockets/named pipes)",
      },
      {
        profile: hasWebSocket ? "loopback_websocket" : "loopback_http",
        available: hasWebSocket,
        description: hasWebSocket
          ? "Loopback WebSocket"
          : "Loopback HTTP",
      },
      {
        profile: "loopback_http",
        available: true,
        description: "Loopback HTTP (compatibility baseline)",
      },
    ];
  }
}
