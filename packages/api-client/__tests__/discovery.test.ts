/**
 * Gateway discovery tests.
 *
 * Tests discovery logic against healthy, missing, and incompatible gateway fixtures.
 * Uses mock fetch to simulate gateway responses without requiring a real server.
 */

import { describe, it, expect, beforeEach, afterEach } from "@jest/globals";

// Mock fetch globally
const originalFetch = global.fetch;

function mockFetch(responses: Map<string, { ok: boolean; status: number; json?: () => Promise<any> }>) {
  global.fetch = jest.fn(async (url: string) => {
    const response = responses.get(url);
    if (!response) {
      throw new Error(`No mock for URL: ${url}`);
    }
    return {
      ok: response.ok,
      status: response.status,
      json: response.json || (async () => ({})),
    } as Response;
  }) as jest.MockedFunction<typeof global.fetch>;
}

function restoreFetch() {
  global.fetch = originalFetch;
}

describe("gateway discovery", () => {
  beforeEach(() => {
    jest.useFakeTimers();
  });

  afterEach(() => {
    restoreFetch();
    jest.useRealTimers();
  });

  describe("probeHealth", () => {
    it("returns healthy when /healthz returns 200 with ok status", async () => {
      const { probeHealth } = await import("../src/discovery");
      mockFetch(
        new Map([
          [
            "http://127.0.0.1:2468/healthz",
            {
              ok: true,
              status: 200,
              json: async () => ({ status: "ok" }),
            },
          ],
        ]),
      );
      const result = await probeHealth("http://127.0.0.1:2468");
      expect(result.healthy).toBe(true);
      expect(result.error).toBeNull();
    });

    it("returns unhealthy when /healthz returns non-200", async () => {
      const { probeHealth } = await import("../src/discovery");
      mockFetch(
        new Map([
          [
            "http://127.0.0.1:2468/healthz",
            { ok: false, status: 503 },
          ],
        ]),
      );
      const result = await probeHealth("http://127.0.0.1:2468");
      expect(result.healthy).toBe(false);
      expect(result.error).toContain("503");
    });

    it("returns unhealthy when /healthz returns unexpected status", async () => {
      const { probeHealth } = await import("../src/discovery");
      mockFetch(
        new Map([
          [
            "http://127.0.0.1:2468/healthz",
            {
              ok: true,
              status: 200,
              json: async () => ({ status: "degraded" }),
            },
          ],
        ]),
      );
      const result = await probeHealth("http://127.0.0.1:2468");
      expect(result.healthy).toBe(false);
      expect(result.error).toContain("degraded");
    });

    it("returns unhealthy when /healthz is unreachable", async () => {
      const { probeHealth } = await import("../src/discovery");
      global.fetch = jest.fn(async () => {
        throw new Error("Connection refused");
      }) as jest.MockedFunction<typeof global.fetch>;
      const result = await probeHealth("http://127.0.0.1:2468");
      expect(result.healthy).toBe(false);
      expect(result.error).toContain("Connection refused");
    });
  });

  describe("probeCapabilities", () => {
    it("returns capabilities when /api/v1/capabilities returns 200", async () => {
      const { probeCapabilities } = await import("../src/discovery");
      const mockCaps = {
        schema_version: "v1",
        gateway_version: "1.0.0",
        supported_api_versions: ["v1"],
        transports: [],
        features: [],
        auth_modes: ["none"],
        max_event_page_size: 100,
        max_terminal_frame_batch: 50,
      };
      mockFetch(
        new Map([
          [
            "http://127.0.0.1:2468/api/v1/capabilities",
            {
              ok: true,
              status: 200,
              json: async () => mockCaps,
            },
          ],
        ]),
      );
      const result = await probeCapabilities("http://127.0.0.1:2468");
      expect(result).toEqual(mockCaps);
    });

    it("returns null when /api/v1/capabilities returns 404", async () => {
      const { probeCapabilities } = await import("../src/discovery");
      mockFetch(
        new Map([
          [
            "http://127.0.0.1:2468/api/v1/capabilities",
            { ok: false, status: 404 },
          ],
        ]),
      );
      const result = await probeCapabilities("http://127.0.0.1:2468");
      expect(result).toBeNull();
    });

    it("returns null when /api/v1/capabilities is unreachable", async () => {
      const { probeCapabilities } = await import("../src/discovery");
      global.fetch = jest.fn(async () => {
        throw new Error("Network error");
      }) as jest.MockedFunction<typeof global.fetch>;
      const result = await probeCapabilities("http://127.0.0.1:2468");
      expect(result).toBeNull();
    });
  });

  describe("discoverGateway", () => {
    it("returns compatible when both health and capabilities succeed", async () => {
      const { discoverGateway } = await import("../src/discovery");
      mockFetch(
        new Map([
          [
            "http://127.0.0.1:2468/healthz",
            {
              ok: true,
              status: 200,
              json: async () => ({ status: "ok" }),
            },
          ],
          [
            "http://127.0.0.1:2468/api/v1/capabilities",
            {
              ok: true,
              status: 200,
              json: async () => ({
                schema_version: "v1",
                gateway_version: "1.0.0",
                supported_api_versions: ["v1"],
                transports: [],
                features: [],
                auth_modes: ["none"],
                max_event_page_size: 100,
                max_terminal_frame_batch: 50,
              }),
            },
          ],
        ]),
      );
      const result = await discoverGateway("http://127.0.0.1:2468");
      expect(result.healthy).toBe(true);
      expect(result.compatible).toBe(true);
      expect(result.error).toBeNull();
      expect(result.capabilities).not.toBeNull();
    });

    it("returns incompatible when capabilities endpoint is missing", async () => {
      const { discoverGateway } = await import("../src/discovery");
      mockFetch(
        new Map([
          [
            "http://127.0.0.1:2468/healthz",
            {
              ok: true,
              status: 200,
              json: async () => ({ status: "ok" }),
            },
          ],
          [
            "http://127.0.0.1:2468/api/v1/capabilities",
            { ok: false, status: 404 },
          ],
        ]),
      );
      const result = await discoverGateway("http://127.0.0.1:2468");
      expect(result.healthy).toBe(true);
      expect(result.compatible).toBe(false);
      expect(result.error).toContain("unreachable");
    });

    it("returns unhealthy when health check fails", async () => {
      const { discoverGateway } = await import("../src/discovery");
      mockFetch(
        new Map([
          [
            "http://127.0.0.1:2468/healthz",
            { ok: false, status: 503 },
          ],
        ]),
      );
      const result = await discoverGateway("http://127.0.0.1:2468");
      expect(result.healthy).toBe(false);
      expect(result.compatible).toBe(false);
      expect(result.capabilities).toBeNull();
    });
  });

  describe("validateGateway", () => {
    it("returns true for healthy compatible gateway", async () => {
      const { validateGateway } = await import("../src/discovery");
      mockFetch(
        new Map([
          [
            "http://127.0.0.1:2468/healthz",
            {
              ok: true,
              status: 200,
              json: async () => ({ status: "ok" }),
            },
          ],
          [
            "http://127.0.0.1:2468/api/v1/capabilities",
            {
              ok: true,
              status: 200,
              json: async () => ({
                schema_version: "v1",
                gateway_version: "1.0.0",
                supported_api_versions: ["v1"],
                transports: [],
                features: [],
                auth_modes: ["none"],
                max_event_page_size: 100,
                max_terminal_frame_batch: 50,
              }),
            },
          ],
        ]),
      );
      const result = await validateGateway("http://127.0.0.1:2468");
      expect(result).toBe(true);
    });

    it("returns false for unhealthy gateway", async () => {
      const { validateGateway } = await import("../src/discovery");
      global.fetch = jest.fn(async () => {
        throw new Error("Connection refused");
      }) as jest.MockedFunction<typeof global.fetch>;
      const result = await validateGateway("http://127.0.0.1:2468");
      expect(result).toBe(false);
    });
  });

  describe("discoverGatewayWithFallback", () => {
    it("does not probe 0.0.0.0 in the default fallback list", async () => {
      const { discoverGatewayWithFallback } = await import("../src/discovery");
      const probedUrls: string[] = [];
      global.fetch = jest.fn(async (url: string) => {
        probedUrls.push(url);
        throw new Error("Connection refused");
      }) as jest.MockedFunction<typeof global.fetch>;

      const result = await discoverGatewayWithFallback();

      expect(result.healthy).toBe(false);
      expect(probedUrls.some((url) => url.includes("0.0.0.0"))).toBe(false);
    });

    it("tries fallback URLs until one succeeds", async () => {
      const { discoverGatewayWithFallback } = await import("../src/discovery");
      let callCount = 0;
      global.fetch = jest.fn(async (url: string) => {
        callCount++;
        if (url.includes("127.0.0.1:2468")) {
          throw new Error("Connection refused");
        }
        if (url.includes("localhost:2468")) {
          if (url.includes("capabilities")) {
            return {
              ok: true,
              status: 200,
              json: async () => ({
                schema_version: "v1",
                gateway_version: "1.0.0",
                supported_api_versions: ["v1"],
                transports: [],
                features: [],
                auth_modes: ["none"],
                max_event_page_size: 100,
                max_terminal_frame_batch: 50,
              }),
            } as Response;
          }
          return {
            ok: true,
            status: 200,
            json: async () => ({ status: "ok" }),
          } as Response;
        }
        return { ok: false, status: 404 } as Response;
      }) as jest.MockedFunction<typeof global.fetch>;
      const result = await discoverGatewayWithFallback();
      expect(callCount).toBeGreaterThanOrEqual(2);
      expect(result.healthy).toBe(true);
      expect(result.compatible).toBe(true);
    });
  });
});
