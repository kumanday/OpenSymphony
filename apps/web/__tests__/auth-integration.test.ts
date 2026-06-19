/**
 * @jest-environment node
 *
 * Web auth integration URL-sync coverage (COE-420 review feedback).
 *
 * The auth integration must keep its `HostedAuthClient` base URI in sync with
 * the active transport: when the shell switches gateway URLs
 * (`onGatewayUrlChanged` -> `setGatewayUrl`), subsequent login/logout/session
 * calls must target the new gateway, and `applySession` must build the
 * transport from the current URL rather than a captured default. These tests
 * inject a `fetchImpl` that records the request URL and a fake transport
 * factory that records the URL/token so the retargeting is observable without
 * a browser or network.
 */

import { createWebAuthIntegration } from "../src/auth-integration";
import { schemaVersionV1, type LoginResponse } from "@opensymphony/gateway-schema";

function loginResponse(token: string): LoginResponse {
  return {
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
}

function okJson(body: unknown): Response {
  const text = JSON.stringify(body);
  return {
    ok: true,
    status: 200,
    statusText: "OK",
    json: async () => JSON.parse(text),
    text: async () => text,
  } as Response;
}

function noContent(): Response {
  return { ok: true, status: 204, statusText: "No Content", text: async () => "" } as Response;
}

describe("web auth integration gateway URL sync (COE-420)", () => {
  it("targets the initial gateway URL for login", async () => {
    const urls: string[] = [];
    const fetchImpl = jest.fn(async (input: RequestInfo | URL) => {
      urls.push(String(input));
      return okJson(loginResponse("sess-1"));
    }) as unknown as typeof fetch;

    const integration = createWebAuthIntegration("http://alpha.local", {
      buildTransport: () => ({} as never),
      fetchImpl,
    });

    const token = await integration.login({
      email: "admin@example.com",
      password: "pw-admin",
      organizationSlug: "acme",
    });

    expect(token).toBe("sess-1");
    expect(urls).toEqual(["http://alpha.local/api/v1/auth/login"]);
  });

  it("retargets login to the new gateway URL after setGatewayUrl", async () => {
    const urls: string[] = [];
    const fetchImpl = jest.fn(async (input: RequestInfo | URL) => {
      urls.push(String(input));
      return okJson(loginResponse("sess-2"));
    }) as unknown as typeof fetch;

    const integration = createWebAuthIntegration("http://alpha.local", {
      buildTransport: () => ({} as never),
      fetchImpl,
    });

    await integration.login({
      email: "admin@example.com",
      password: "pw-admin",
      organizationSlug: "acme",
    });

    integration.setGatewayUrl("http://beta.local");

    await integration.login({
      email: "admin@example.com",
      password: "pw-admin",
      organizationSlug: "acme",
    });

    expect(urls).toEqual([
      "http://alpha.local/api/v1/auth/login",
      "http://beta.local/api/v1/auth/login",
    ]);
  });

  it("applySession builds the transport from the current gateway URL, not a captured default", async () => {
    const built: Array<{ url: string; token?: string }> = [];
    const fetchImpl = jest.fn(async () => noContent()) as unknown as typeof fetch;

    const integration = createWebAuthIntegration("http://alpha.local", {
      buildTransport: (url, token) => {
        built.push({ url, token });
        return {} as never;
      },
      fetchImpl,
    });

    integration.setGatewayUrl("http://beta.local");

    await integration.applySession("sess-3");

    expect(built).toEqual([{ url: "http://beta.local", token: "sess-3" }]);
  });

  it("logout targets the current gateway URL and clears the session", async () => {
    const urls: string[] = [];
    let next = "sess-4";
    const fetchImpl = jest.fn(async (input: RequestInfo | URL) => {
      urls.push(String(input));
      if (String(input).endsWith("/login")) {
        return okJson(loginResponse(next));
      }
      return noContent();
    }) as unknown as typeof fetch;

    const integration = createWebAuthIntegration("http://alpha.local", {
      buildTransport: () => ({} as never),
      fetchImpl,
    });

    await integration.login({
      email: "admin@example.com",
      password: "pw-admin",
      organizationSlug: "acme",
    });
    integration.setGatewayUrl("http://beta.local");
    await integration.logout();

    expect(urls).toEqual([
      "http://alpha.local/api/v1/auth/login",
      "http://beta.local/api/v1/auth/logout",
    ]);
  });
});