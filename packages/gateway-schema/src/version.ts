/** Gateway API schema version constant. */
export const GATEWAY_SCHEMA_VERSION = "1.0.0" as const;

/** Semantic version wrapper used in every versioned payload. */
export interface SchemaVersion {
  major: number;
  minor: number;
  patch: number;
}

/** Factory for a v1 schema version. */
export function schemaVersionV1(): SchemaVersion {
  return { major: 1, minor: 0, patch: 0 };
}

/** Convert a SchemaVersion to a dotted string. */
export function schemaVersionToString(v: SchemaVersion): string {
  return `${v.major}.${v.minor}.${v.patch}`;
}

/** Parse a dotted version string into a SchemaVersion. */
export function schemaVersionFromString(s: string): SchemaVersion {
  const parts = s.split(".");
  if (parts.length !== 3 || parts.some((p) => p === "")) {
    throw new Error(`Invalid schema version string: ${s}`);
  }
  const [major, minor, patch] = parts.map(Number);
  if ([major, minor, patch].some((n) => !Number.isFinite(n))) {
    throw new Error(`Invalid schema version string: ${s}`);
  }
  return { major, minor, patch };
}
