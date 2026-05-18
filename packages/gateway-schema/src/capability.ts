import type { SchemaVersion } from "./version.js";
import type { TransportProfile } from "./transport.js";

export type AuthMode =
  | "none"
  | "api_key"
  | "bearer_token"
  | "subscription_oauth";

export interface TransportCapability {
  transport: TransportProfile;
  modes: string[];
  supported_encodings: string[];
  bidirectional: boolean;
}

export interface FeatureCapability {
  feature: string;
  available: boolean;
  requires_auth: boolean;
  requires_plan?: string;
}

/** Capability discovery response. */
export interface GatewayCapabilities {
  schema_version: SchemaVersion;
  gateway_version: string;
  supported_api_versions: string[];
  transports: TransportCapability[];
  features: FeatureCapability[];
  auth_modes: AuthMode[];
  max_event_page_size: number;
  max_terminal_frame_batch: number;
}
