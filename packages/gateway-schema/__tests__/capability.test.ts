import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import type { HarnessProfileCapability, HarnessRunCapability } from "@opensymphony/gateway-schema";

test("Rust and TypeScript share ACP profile and negotiated run projections", () => {
  const fixture = JSON.parse(readFileSync(resolve(__dirname, "fixtures/acp-capability.json"), "utf8")) as {
    profile: HarnessProfileCapability; run: HarnessRunCapability;
  };
  const ready: boolean = fixture.profile.preflight_ready;
  const negotiated: boolean = fixture.run.session_restore;
  expect(ready).toBe(false);
  expect(negotiated).toBe(true);
  expect(fixture.profile.operations?.[0].operation_id).toBe("fixture.echo");
  expect(fixture.run.operations?.[0].deadline_ms).toBe(5000);
  expect(fixture.run).not.toHaveProperty("input_tokens");
  expect(fixture.profile).not.toHaveProperty("command");
  expect(JSON.parse(JSON.stringify(fixture))).toEqual(fixture);
});
