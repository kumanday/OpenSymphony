/**
 * Unit tests for connection profile types.
 *
 * Tests default profiles, profile creation, serialization, and kind-based logic.
 */

import { describe, it, expect } from "@jest/globals";
import {
  defaultProfiles,
  createProfile,
  type ConnectionProfile,
  type ConnectionProfileKind,
  type LocalDaemonProfile,
  type SupervisedLocalDaemonProfile,
  type EmbeddedHostProfile,
  type ExternalGatewayProfile,
  type HostedGatewayProfile,
} from "@opensymphony/gateway-schema";

describe("defaultProfiles", () => {
  it("returns exactly five profile kinds", () => {
    const profiles = defaultProfiles();
    expect(profiles).toHaveLength(5);
  });

  it("includes a local_daemon profile with loopback_http transport", () => {
    const profiles = defaultProfiles();
    const localDaemon = profiles.find((p) => p.kind === "local_daemon");
    expect(localDaemon).toBeDefined();
    expect(localDaemon!.transport).toBe("loopback_http");
    expect(localDaemon!.managed).toBe(false);
    expect(localDaemon!.gatewayUrl).toBe("http://127.0.0.1:8080");
  });

  it("includes a supervised_local_daemon profile with auto-restart", () => {
    const profiles = defaultProfiles();
    const supervised = profiles.find(
      (p) => p.kind === "supervised_local_daemon",
    ) as SupervisedLocalDaemonProfile | undefined;
    expect(supervised).toBeDefined();
    expect(supervised!.managed).toBe(true);
    expect(supervised!.autoRestart).toBe(true);
    expect(supervised!.startupTimeoutSecs).toBe(30);
  });

  it("includes an embedded_host profile with in_process_channel transport", () => {
    const profiles = defaultProfiles();
    const embedded = profiles.find(
      (p) => p.kind === "embedded_host",
    ) as EmbeddedHostProfile | undefined;
    expect(embedded).toBeDefined();
    expect(embedded!.managed).toBe(true);
    expect(embedded!.transport).toBe("in_process_channel");
  });

  it("includes an external_gateway profile with probeOnConnect", () => {
    const profiles = defaultProfiles();
    const external = profiles.find(
      (p) => p.kind === "external_gateway",
    ) as ExternalGatewayProfile | undefined;
    expect(external).toBeDefined();
    expect(external!.managed).toBe(false);
    expect(external!.probeOnConnect).toBe(true);
  });

  it("includes a hosted_gateway profile with websocket transport", () => {
    const profiles = defaultProfiles();
    const hosted = profiles.find(
      (p) => p.kind === "hosted_gateway",
    ) as HostedGatewayProfile | undefined;
    expect(hosted).toBeDefined();
    expect(hosted!.managed).toBe(false);
    expect(hosted!.transport).toBe("websocket");
  });

  it("all profiles start inactive", () => {
    const profiles = defaultProfiles();
    profiles.forEach((p) => expect(p.active).toBe(false));
  });
});

describe("createProfile", () => {
  it("creates a local_daemon profile with defaults", () => {
    const profile = createProfile("local_daemon") as LocalDaemonProfile;
    expect(profile.kind).toBe("local_daemon");
    expect(profile.managed).toBe(false);
    expect(profile.id).toMatch(/^local_daemon-\d+-\d+$/);
    expect(profile.active).toBe(false);
  });

  it("creates a supervised_local_daemon profile with defaults", () => {
    const profile = createProfile(
      "supervised_local_daemon",
    ) as SupervisedLocalDaemonProfile;
    expect(profile.kind).toBe("supervised_local_daemon");
    expect(profile.managed).toBe(true);
    expect(profile.daemonPath).toBe("");
    expect(profile.daemonArgs).toEqual([]);
    expect(profile.daemonEnv).toEqual({});
  });

  it("creates an embedded_host profile with defaults", () => {
    const profile = createProfile("embedded_host") as EmbeddedHostProfile;
    expect(profile.kind).toBe("embedded_host");
    expect(profile.managed).toBe(true);
    expect(profile.transport).toBe("in_process_channel");
  });

  it("creates an external_gateway profile with defaults", () => {
    const profile = createProfile(
      "external_gateway",
    ) as ExternalGatewayProfile;
    expect(profile.kind).toBe("external_gateway");
    expect(profile.managed).toBe(false);
    expect(profile.probeOnConnect).toBe(true);
  });

  it("creates a hosted_gateway profile with defaults", () => {
    const profile = createProfile("hosted_gateway") as HostedGatewayProfile;
    expect(profile.kind).toBe("hosted_gateway");
    expect(profile.managed).toBe(false);
    expect(profile.gatewayUrl).toBe("");
    expect(profile.probeOnConnect).toBe(true);
  });

  it("throws for unknown profile kind", () => {
    expect(() => createProfile("unknown" as ConnectionProfileKind)).toThrow(
      "Unknown connection profile kind: unknown",
    );
  });

  it("generates unique IDs based on timestamp", () => {
    const profile1 = createProfile("local_daemon");
    const profile2 = createProfile("local_daemon");
    expect(profile1.id).not.toBe(profile2.id);
  });
});

describe("profile serialization", () => {
  it("serializes and deserializes a local_daemon profile", () => {
    const profile: ConnectionProfile = {
      id: "test-1",
      label: "Test Local",
      kind: "local_daemon",
      active: true,
      gatewayUrl: "http://127.0.0.1:9090",
      transport: "loopback_http",
      managed: false,
    };

    const serialized = JSON.stringify(profile);
    const deserialized: ConnectionProfile = JSON.parse(serialized);

    expect(deserialized.kind).toBe("local_daemon");
    expect(deserialized.gatewayUrl).toBe("http://127.0.0.1:9090");
    expect(deserialized.active).toBe(true);
  });

  it("serializes and deserializes a supervised_local_daemon profile", () => {
    const profile: ConnectionProfile = {
      id: "test-2",
      label: "Test Supervised",
      kind: "supervised_local_daemon",
      active: false,
      gatewayUrl: "http://127.0.0.1:8080",
      transport: "loopback_http",
      managed: true,
      daemonPath: "/usr/local/bin/opensymphony-daemon",
      daemonArgs: ["--verbose"],
      daemonEnv: { LOG_LEVEL: "debug" },
      startupTimeoutSecs: 60,
      autoRestart: false,
    };

    const serialized = JSON.stringify(profile);
    const deserialized: ConnectionProfile = JSON.parse(serialized);

    expect(deserialized.kind).toBe("supervised_local_daemon");
    expect(deserialized.managed).toBe(true);
    expect(deserialized.daemonPath).toBe("/usr/local/bin/opensymphony-daemon");
  });
});
