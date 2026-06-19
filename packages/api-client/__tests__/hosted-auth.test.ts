/**
 * Hosted auth login flow + auth-state classification (COE-420).
 *
 * Exercises the real `HostedAuthClient` login/session/logout network calls
 * against an injected fetch (no mocks of the unit under test) and the shared
 * `authStateFromError` classifier for the expanded auth error code set.
 */

import { HostedAuthClient, GatewayRequestError } from "@opensymphony/api-client";
import {
  authStateFromError,
  schemaVersionV1,
  type LoginResponse,
  type SessionResponse,
} from "@opensymphony/gateway-schema";

function mockResponse(status: number, statusText: string, body: string): Response {
  return {
    ok: status >= 200 && status < 300,
    status,
    statusText,
    json: async () => JSON.parse(body),
    text: async () => body,
  } as Response;
}

function loginResponse(token: string): string {
  const body: LoginResponse = {
    schema_version: schemaVersionV1(),
    session_token: token,
    user: {
      schema_version: schemaVersionV1(),
      user_id: "u-admin",
      email: "admin@example.com",
      display_name: "Admin User",
      handle: "admin",
    },
    organization: {
      schema_version: schemaVersionV1(),
      organization_id: "o-acme",
      slug: "acme",
      display_name: "Acme",
    },
    role: "admin",
    expires_at: "2025-12-31T00:00:00Z",
  };
  return JSON.stringify(body);
}

function sessionResponse(): string {
  const body: SessionResponse = {
    schema_version: schemaVersionV1(),
    user: {
      schema_version: schemaVersionV1(),
      user_id: "u-admin",
      email: "admin@example.com",
      display_name: "Admin User",
      handle: "admin",
    },
    organization: {
      schema_version: schemaVersionV1(),
      organization_id: "o-acme",
      slug: "acme",
      display_name: "Acme",
    },
    role: "admin",
    expires_at: "2025-12-31T00:00:00Z",
  };
  return JSON.stringify(body);
}

describe("HostedAuthClient login flow (COE-420)", () => {
  it("login posts credentials, holds the session token, and resolves identity + tenant", async () => {
    const calls: RequestInit[] = [];
    const fetchImpl = jest.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      calls.push(init ?? {});
      return mockResponse(200, "OK", loginResponse("sess-123"));
    }) as jest.MockedFunction<typeof global.fetch>;

    const client = new HostedAuthClient({
      baseUri: "https://hosted.opensymphony.example/",
      fetchImpl,
    });

    const result = await client.login({
      email: "admin@example.com",
      password: "pw-admin",
      organization_slug: "acme",
    });

    expect(result.session_token).toBe("sess-123");
    expect(result.user.email).toBe("admin@example.com");
    expect(result.organization.slug).toBe("acme");
    expect(result.role).toBe("admin");
    // The client holds the token for subsequent authenticated calls.
    expect(client.token).toBe("sess-123");

    // The login request carries the JSON body and no bearer token.
    expect(calls).toHaveLength(1);
    expect(calls[0].method).toBe("POST");
    expect(JSON.parse(calls[0].body as string)).toEqual({
      email: "admin@example.com",
      password: "pw-admin",
      organization_slug: "acme",
    });
  });

  it("session probes with the bearer token and resolves the identity", async () => {
    const calls: RequestInit[] = [];
    const fetchImpl = jest.fn(async (_input: RequestInfo | URL, init?: RequestInit) => {
      calls.push(init ?? {});
      return mockResponse(200, "OK", sessionResponse());
    }) as jest.MockedFunction<typeof global.fetch>;

    const client = new HostedAuthClient({
      baseUri: "https://hosted.opensymphony.example",
      sessionToken: "sess-123",
      fetchImpl,
    });

    const session = await client.session();
    expect(session.user.email).toBe("admin@example.com");
    expect(session.organization.slug).toBe("acme");

    // The probe is a GET with the bearer token.
    expect(calls).toHaveLength(1);
    expect(calls[0].method).toBe("GET");
    const headers = calls[0].headers as Record<string, string>;
    expect(headers.Authorization).toBe("Bearer sess-123");
  });

  it("logout posts with the bearer token and clears the held token", async () => {
    const calls: RequestInit[] = [];
    const fetchImpl = jest.fn(async (_input: RequestInfo | URL, init?: RequestInit) => {
      calls.push(init ?? {});
      return mockResponse(204, "No Content", "");
    }) as jest.MockedFunction<typeof global.fetch>;

    const client = new HostedAuthClient({
      baseUri: "https://hosted.opensymphony.example",
      sessionToken: "sess-123",
      fetchImpl,
    });

    await client.logout();
    expect(client.token).toBeUndefined();
    expect(calls).toHaveLength(1);
    expect(calls[0].method).toBe("POST");
    const headers = calls[0].headers as Record<string, string>;
    expect(headers.Authorization).toBe("Bearer sess-123");
  });

  it("classifies a 401 login as unauthenticated", async () => {
    const fetchImpl = jest.fn(async () =>
      mockResponse(401, "Unauthorized", '{"error_code":"unauthenticated","message":"bad creds"}'),
    ) as jest.MockedFunction<typeof global.fetch>;

    const client = new HostedAuthClient({
      baseUri: "https://hosted.opensymphony.example",
      fetchImpl,
    });

    await expect(
      client.login({ email: "admin@example.com", password: "wrong" }),
    ).rejects.toMatchObject({
      name: "GatewayRequestError",
      code: "unauthenticated",
      status: 401,
    });
  });

  it("classifies a 403 permission-denial body as unauthorized", async () => {
    const fetchImpl = jest.fn(async () =>
      mockResponse(
        403,
        "Forbidden",
        '{"error_code":"permission_denied","message":"no access to project"}',
      ),
    ) as jest.MockedFunction<typeof global.fetch>;

    const client = new HostedAuthClient({
      baseUri: "https://hosted.opensymphony.example",
      sessionToken: "sess-123",
      fetchImpl,
    });

    await expect(client.session()).rejects.toMatchObject({
      name: "GatewayRequestError",
      code: "unauthorized",
      status: 403,
    });
  });
});

describe("authStateFromError expanded code set (COE-420)", () => {
  it("maps permission_denied and forbidden_resource body codes to unauthorized", () => {
    expect(authStateFromError({ code: "permission_denied" })).toBe("unauthorized");
    expect(authStateFromError({ code: "forbidden_resource" })).toBe("unauthorized");
    expect(authStateFromError({ code: "unauthorized" })).toBe("unauthorized");
  });

  it("maps dev_bypass_disabled to forbidden", () => {
    expect(authStateFromError({ code: "dev_bypass_disabled" })).toBe("forbidden");
    expect(authStateFromError({ code: "forbidden" })).toBe("forbidden");
  });

  it("maps auth_disabled (disabled-mode auth endpoints) to forbidden", () => {
    expect(authStateFromError({ code: "auth_disabled" })).toBe("forbidden");
  });

  it("maps unauthenticated to unauthenticated and leaves unrelated errors open", () => {
    expect(authStateFromError({ code: "unauthenticated" })).toBe("unauthenticated");
    expect(authStateFromError({ code: "unavailable" })).toBe("open");
    expect(authStateFromError(new Error("network down"))).toBe("open");
  });

  it("classifies a thrown GatewayRequestError by its code", () => {
    const unauthorized = new GatewayRequestError("unauthorized", "no permission", 403);
    expect(authStateFromError(unauthorized)).toBe("unauthorized");
    const forbidden = new GatewayRequestError("forbidden", "hard deny", 403);
    expect(authStateFromError(forbidden)).toBe("forbidden");
    const unauthenticated = new GatewayRequestError("unauthenticated", "no token", 401);
    expect(authStateFromError(unauthenticated)).toBe("unauthenticated");
  });
});