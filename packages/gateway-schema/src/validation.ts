/**
 * Runtime validators for gateway schema payloads.
 *
 * TypeScript types are erased at runtime; these helpers guard stream
 * payloads and REST responses so consumers can fail fast on shape
 * mismatches before the typed reducer logic runs.
 */

import type { GatewayEnvelope } from "./envelope.js";
import type { SchemaVersion } from "./version.js";
import { GATEWAY_SCHEMA_VERSION } from "./version.js";

/** Status of a validation command or evidence item. */
export type ValidationStatus =
  | "pending"
  | "running"
  | "passed"
  | "failed"
  | "skipped"
  | "error";

/** A single validation command executed as part of a run. */
export interface ValidationCommand {
  command_id: string;
  command: string;
  status: ValidationStatus;
  started_at?: string;
  finished_at?: string;
  exit_code?: number;
  stdout_summary?: string;
  stderr_summary?: string;
}

/** Evidence attached to a validation outcome. */
export interface ValidationEvidenceItem {
  evidence_id: string;
  label: string;
  status: ValidationStatus;
  summary: string;
  command_id?: string;
  file_path?: string;
  line_number?: number;
  happened_at?: string;
}

/** Validation summary for a run. */
export interface RunValidationSummary {
  schema_version: SchemaVersion;
  run_id: string;
  generated_at: string;
  overall_status: ValidationStatus;
  commands: ValidationCommand[];
  evidence: ValidationEvidenceItem[];
}

/** Return true when the payload's schema_version has valid numeric fields. */
export function isValidSchemaVersion(v: unknown): v is SchemaVersion {
  if (
    typeof v !== "object" ||
    v === null ||
    !("major" in v) ||
    !("minor" in v) ||
    !("patch" in v)
  ) {
    return false;
  }
  const obj = v as SchemaVersion;
  return (
    Number.isFinite(obj.major) &&
    Number.isFinite(obj.minor) &&
    Number.isFinite(obj.patch)
  );
}

/** Throw if the schema version is incompatible. */
export function assertCompatibleSchemaVersion(
  v: unknown,
  label = "payload",
): asserts v is SchemaVersion {
  if (!isValidSchemaVersion(v)) {
    throw new Error(
      `[schema] ${label}: schema_version must be { major, minor, patch }, got ${JSON.stringify(v)}`,
    );
  }
  if (v.major !== 1) {
    throw new Error(
      `[schema] ${label}: unsupported schema major=${v.major} (expected 1)`,
    );
  }
}

export function isValidGatewayEnvelope(envelope: unknown): envelope is GatewayEnvelope {
  if (typeof envelope !== "object" || envelope === null) return false;
  const e = envelope as Record<string, unknown>;
  return (
    isValidSchemaVersion(e.schema_version) &&
    typeof e.cursor === "object" &&
    e.cursor !== null &&
    typeof (e.cursor as Record<string, unknown>).sequence === "number" &&
    typeof (e.cursor as Record<string, unknown>).partition === "string" &&
    typeof e.entity_ref === "object" &&
    e.entity_ref !== null &&
    typeof (e.entity_ref as Record<string, unknown>).kind === "string" &&
    typeof (e.entity_ref as Record<string, unknown>).id === "string" &&
    typeof e.event_kind === "string" &&
    typeof e.emitted_at === "string"
  );
}

export function assertValidGatewayEnvelope(
  envelope: unknown,
  label = "envelope",
): asserts envelope is GatewayEnvelope {
  if (!isValidGatewayEnvelope(envelope)) {
    throw new Error(
      `[schema] ${label}: payload does not match GatewayEnvelope shape`,
    );
  }
  assertCompatibleSchemaVersion(envelope.schema_version, label);
}

/** Validate a batch of envelopes; returns array of indexes that failed (no throw). */
export function validateEnvelopeBatch(
  batch: unknown[],
): number[] {
  const failed: number[] = [];
  for (let i = 0; i < batch.length; i++) {
    if (!isValidGatewayEnvelope(batch[i])) {
      failed.push(i);
    }
  }
  return failed;
}

/** Throw if any envelope in the batch fails validation. */
export function assertValidEnvelopeBatch(
  batch: unknown[],
  label = "batch",
): void {
  const failed = validateEnvelopeBatch(batch);
  if (failed.length > 0) {
    throw new Error(
      `[schema] ${label}: envelopes at indices ${failed.join(",")} failed validation`,
    );
  }
}

/** Parse and validate a JSON string as a GatewayEnvelope. */
export function parseGatewayEnvelope(
  raw: string,
  label = "raw JSON",
): GatewayEnvelope {
  const parsed = JSON.parse(raw);
  assertValidGatewayEnvelope(parsed, label);
  return parsed;
}

/** Return the gateway schema version string constant for reference. */
export function getGatewaySchemaVersion(): string {
  return GATEWAY_SCHEMA_VERSION;
}