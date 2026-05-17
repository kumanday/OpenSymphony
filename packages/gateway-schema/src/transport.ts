export type TransportProfile =
  | "in_process_channel"
  | "native_ipc"
  | "tauri_channel"
  | "loopback_http"
  | "loopback_websocket"
  | "sse"
  | "websocket"
  | "json_rpc_over_websocket";

/** Transport recommendation metadata. */
export interface TransportRecommendation {
  profile: TransportProfile;
  priority: number;
  description: string;
  expected_latency_ms: number;
  expected_throughput_kbps: number;
  reconnect_support: boolean;
  replay_support: boolean;
  binary_frame_support: boolean;
  auth_required: boolean;
}
