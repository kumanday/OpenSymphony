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
  harnesses?: HarnessCapability[];
  harness_profiles?: HarnessProfileCapability[];
  features: FeatureCapability[];
  auth_modes: AuthMode[];
  max_event_page_size: number;
  max_terminal_frame_batch: number;
}

export interface HarnessCapability {
  kind: string;
  display_name: string;
  available: boolean;
  adapter_contract_version: string;
  runtime_contract_version: string | null;
  actions: {
    start_run: boolean;
    send_user_message: boolean;
    retry: boolean;
    cancel: boolean;
    pause: boolean;
    resume: boolean;
    approve: boolean;
    reject: boolean;
    comment: boolean;
  };
  event_streams: {
    runtime_events: boolean;
    terminal_frames: boolean;
    replay_from_cursor: boolean;
    raw_payload_refs: boolean;
    delivery_modes: string[];
  };
  approvals: { tool_approval: boolean; human_decision: boolean; policy_metadata: boolean };
  model_settings: {
    api_compatible_settings: boolean;
    subscription_credentials: boolean;
    per_run_overrides: boolean;
    credential_reference_kinds: string[];
  };
  transport: { protocol: string; modes: string[]; local: boolean; remote: boolean };
  cancellation: { cancel_run: boolean; force_stop: boolean; acknowledges_cancel: boolean };
  pause_resume: { pause: boolean; resume: boolean };
  history: {
    fetch_history: boolean;
    reconcile_after_ready: boolean;
    reconnect_and_replay: boolean;
    preserve_unknown_events: boolean;
  };
  notes: string[];
  feature_gaps: string[];
}

/** Local preflight does not prove successful agent authentication. */
export interface HarnessProfileCapability {
  harness: string;
  profile_id: string;
  preflight_ready: boolean;
  unavailable_reason: string | null;
}

export interface HarnessRunCapability {
  harness: string;
  profile_id: string;
  protocol: string;
  protocol_version: number;
  rpc: string;
  encoding: string;
  framing: string;
  carrier: string;
  session_restore: boolean;
  history_replay: boolean;
  model_selection: boolean;
  cancellation: boolean;
  operator_responses: boolean;
}
