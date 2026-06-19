/**
 * Gateway request error classification for COE-419.
 *
 * Exercises the real `HttpGatewayTransport.fetchJson` HTTP 401/403 mapping
 * and the shared `authStateFromError` classifier (no mocks of the units
 * under test).
 */

import {
  HttpGatewayTransport,
  WebSocketTransport,
  MockGatewayTransport,
  GatewayRequestError,
  isGatewayRequestError,
  authErrorCodeForStatus,
} from "@opensymphony/api-client";
import { authStateFromError } from "@opensymphony/gateway-schema";
import { schemaVersionV1 } from "@opensymphony/gateway-schema";
import type { GatewayCapabilities } from "@opensymphony/gateway-schema";

const FIXTURE_CAPABILITIES: GatewayCapabilities = {
  schema_version: schemaVersionV1(),
  gateway_version: "error-test",
  supported_api_versions: ["1.0.0"],
  transports: [{ transport: "loopback_http", modes: ["json"], supported_encodings: ["utf-8"], bidirectional: false }],
  features: [{ feature: "task_graph", available: true, requires_auth: false }],
  auth_modes: ["none"],
  max_event_page_size: 1000,
  max_terminal_frame_batch: 500,
};

function mockResponse(status: number, statusText: string, body: string): Response {
  return {
    ok: status >= 200 && status < 300,
    status,
    statusText,
    json: async () => JSON.parse(body),
    text: async () => body,
  } as Response;
}

describe("GatewayRequestError classification", () => {
  describe("authErrorCodeForStatus", () => {
    it("maps 401 to unauthenticated", () => {
      expect(authErrorCodeForStatus(401)).toBe("unauthenticated");
    });
    it("maps a bare 403 (no permission body signal) to forbidden", () => {
      expect(authErrorCodeForStatus(403)).toBe("forbidden");
      expect(authErrorCodeForStatus(403, undefined)).toBe("forbidden");
      expect(authErrorCodeForStatus(403, "not json")).toBe("forbidden");
    });
    it("maps a 403 with an explicit unauthorized body code to unauthorized", () => {
      expect(authErrorCodeForStatus(403, { error_code: "unauthorized" })).toBe("unauthorized");
      expect(authErrorCodeForStatus(403, { code: "permission_denied" })).toBe("unauthorized");
      expect(authErrorCodeForStatus(403, { error_code: "forbidden_resource" })).toBe("unauthorized");
    });
    it("maps a 403 with an unrelated body code to forbidden", () => {
      expect(authErrorCodeForStatus(403, { error_code: "rate_limited" })).toBe("forbidden");
    });
    it("returns undefined for non-auth statuses", () => {
      expect(authErrorCodeForStatus(500)).toBeUndefined();
      expect(authErrorCodeForStatus(404)).toBeUndefined();
      expect(authErrorCodeForStatus(200)).toBeUndefined();
    });
  });

  describe("HttpGatewayTransport HTTP mapping", () => {
    const originalFetch = global.fetch;
    afterEach(() => {
      global.fetch = originalFetch;
    });

    it("throws a GatewayRequestError with unauthenticated code on HTTP 401", async () => {
      global.fetch = jest.fn(async () =>
        mockResponse(401, "Unauthorized", '{"error":"missing token"}'),
      ) as jest.MockedFunction<typeof global.fetch>;

      const transport = new HttpGatewayTransport({ baseUri: "http://localhost:8080" });
      await expect(transport.health()).rejects.toMatchObject({
        code: "unauthenticated",
        status: 401,
        name: "GatewayRequestError",
      });
    });

    it("throws a GatewayRequestError with forbidden code on HTTP 403", async () => {
      global.fetch = jest.fn(async () =>
        mockResponse(403, "Forbidden", '{"error":"no access"}'),
      ) as jest.MockedFunction<typeof global.fetch>;

      const transport = new HttpGatewayTransport({ baseUri: "http://localhost:8080" });
      await expect(transport.snapshot()).rejects.toMatchObject({
        code: "forbidden",
        status: 403,
      });
    });

    it("classifies a 403 with an explicit unauthorized body code as unauthorized", async () => {
      global.fetch = jest.fn(async () =>
        mockResponse(403, "Forbidden", '{"error_code":"unauthorized","message":"no permission"}'),
      ) as jest.MockedFunction<typeof global.fetch>;

      const transport = new HttpGatewayTransport({ baseUri: "http://localhost:8080" });
      await expect(transport.snapshot()).rejects.toMatchObject({
        code: "unauthorized",
        status: 403,
      });
    });

    it("throws a plain Error (not GatewayRequestError) on HTTP 500", async () => {
      global.fetch = jest.fn(async () =>
        mockResponse(500, "Internal Server Error", '{"error":"boom"}'),
      ) as jest.MockedFunction<typeof global.fetch>;

      const transport = new HttpGatewayTransport({ baseUri: "http://localhost:8080" });
      const rejection = transport.health();
      await expect(rejection).rejects.toThrow(/HTTP 500/);
      try {
        await rejection;
      } catch (error) {
        expect(isGatewayRequestError(error)).toBe(false);
      }
    });
  });

  describe("WebSocketTransport HTTP handshake mapping", () => {
    const originalFetch = global.fetch;
    afterEach(() => {
      global.fetch = originalFetch;
    });

    it("throws a GatewayRequestError with unauthenticated code on HTTP 401", async () => {
      global.fetch = jest.fn(async () =>
        mockResponse(401, "Unauthorized", '{"error":"missing token"}'),
      ) as jest.MockedFunction<typeof global.fetch>;

      const transport = new WebSocketTransport({ baseUri: "http://localhost:8080" });
      await expect(transport.health()).rejects.toMatchObject({
        code: "unauthenticated",
        status: 401,
        name: "GatewayRequestError",
      });
    });

    it("includes the raw response body in the WebSocketTransport error message", async () => {
      global.fetch = jest.fn(async () =>
        mockResponse(401, "Unauthorized", '{"error":"missing token"}'),
      ) as jest.MockedFunction<typeof global.fetch>;

      const transport = new WebSocketTransport({ baseUri: "http://localhost:8080" });
      try {
        await transport.health();
        throw new Error("expected health() to reject");
      } catch (error) {
        expect(isGatewayRequestError(error)).toBe(true);
        expect((error as Error).message).toContain('{"error":"missing token"}');
      }
    });

    it("throws a GatewayRequestError with forbidden code on a bare HTTP 403", async () => {
      global.fetch = jest.fn(async () =>
        mockResponse(403, "Forbidden", '{"error":"no access"}'),
      ) as jest.MockedFunction<typeof global.fetch>;

      const transport = new WebSocketTransport({ baseUri: "http://localhost:8080" });
      await expect(transport.snapshot()).rejects.toMatchObject({
        code: "forbidden",
        status: 403,
      });
    });

    it("classifies a 403 with an explicit unauthorized body code as unauthorized", async () => {
      global.fetch = jest.fn(async () =>
        mockResponse(403, "Forbidden", '{"error_code":"unauthorized","message":"no permission"}'),
      ) as jest.MockedFunction<typeof global.fetch>;

      const transport = new WebSocketTransport({ baseUri: "http://localhost:8080" });
      await expect(transport.snapshot()).rejects.toMatchObject({
        code: "unauthorized",
        status: 403,
      });
    });

    it("throws a plain Error (not GatewayRequestError) on HTTP 500", async () => {
      global.fetch = jest.fn(async () =>
        mockResponse(500, "Internal Server Error", '{"error":"boom"}'),
      ) as jest.MockedFunction<typeof global.fetch>;

      const transport = new WebSocketTransport({ baseUri: "http://localhost:8080" });
      const rejection = transport.health();
      await expect(rejection).rejects.toThrow(/HTTP 500/);
      try {
        await rejection;
      } catch (error) {
        expect(isGatewayRequestError(error)).toBe(false);
      }
    });
  });

  describe("authStateFromError classifier", () => {
    it("classifies unauthenticated errors", () => {
      expect(authStateFromError(new GatewayRequestError("unauthenticated", "x", 401))).toBe("unauthenticated");
    });
    it("classifies forbidden errors", () => {
      expect(authStateFromError(new GatewayRequestError("forbidden", "x", 403))).toBe("forbidden");
    });
    it("classifies unauthorized errors", () => {
      expect(authStateFromError(new GatewayRequestError("unauthorized", "x", 403))).toBe("unauthorized");
    });
    it("returns open for non-auth errors", () => {
      expect(authStateFromError(new Error("boom"))).toBe("open");
      expect(authStateFromError(new GatewayRequestError("unavailable", "x", 500))).toBe("open");
      expect(authStateFromError(null)).toBe("open");
    });
  });

  describe("MockGatewayTransport auth simulation", () => {
    it("throws a classified auth error from snapshot while health succeeds", async () => {
      const transport = new MockGatewayTransport({
        health: FIXTURE_CAPABILITIES,
        authFailure: { code: "unauthenticated", methods: ["snapshot"] },
      });
      // health succeeds so the shell can read auth_modes
      await expect(transport.health()).resolves.toBeDefined();
      await expect(transport.snapshot()).rejects.toMatchObject({ code: "unauthenticated" });
    });

    it("clearAuthFailure lets subsequent reads succeed", async () => {
      const transport = new MockGatewayTransport({
        health: FIXTURE_CAPABILITIES,
        authFailure: { code: "forbidden", methods: ["snapshot"] },
      });
      await expect(transport.snapshot()).rejects.toMatchObject({ code: "forbidden" });
      transport.clearAuthFailure();
      await expect(transport.snapshot()).resolves.toBeDefined();
    });
  });
});