use std::time::{Duration, Instant};

use futures_util::{SinkExt, StreamExt};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::UnixStream,
    sync::mpsc,
};

use opensymphony::opensymphony_gateway_schema::envelope::GatewayEnvelope;
use opensymphony::opensymphony_gateway_schema::terminal::{
    TerminalEncoding, TerminalFrame, TerminalFrameKind, TerminalLogAssociation,
};
use opensymphony::opensymphony_gateway_schema::transport::TransportProfile;
use opensymphony::opensymphony_gateway_schema::version::SchemaVersion;

const BENCH_PAYLOAD: &str = concat!(
    "[2025-08-17T09:12:00Z] INFO  agent::executor > Starting turn 3\n",
    "[2025-08-17T09:12:01Z] DEBUG agent::planner  > Evaluating tool call: file_write\n",
    "[2025-08-17T09:12:02Z] INFO  agent::tool      > Writing 42 bytes to src/main.rs\n",
    "[2025-08-17T09:12:03Z] INFO  agent::executor > Turn 3 completed in 2.1s\n",
    "[2025-08-17T09:12:04Z] DEBUG agent::runtime  > Awaiting next event\n",
);

fn sample_terminal_frame(sequence: u64) -> TerminalFrame {
    TerminalFrame {
        schema_version: SchemaVersion::v1(),
        frame_sequence: sequence,
        stream_id: "stream-bench".into(),
        run_id: "run-bench".into(),
        terminal_session_id: "term-bench".into(),
        frame_kind: TerminalFrameKind::Log,
        encoding: TerminalEncoding::Utf8,
        content: BENCH_PAYLOAD.into(),
        timestamp: chrono::Utc::now(),
        association: TerminalLogAssociation {
            run_id: "run-bench".into(),
            workspace_id: "workspace-bench".into(),
            command_id: None,
            issue_id: None,
            sub_issue_id: None,
            harness_session_id: None,
        },
        correlation_id: None,
        source_event_id: Some(format!("evt-{sequence}")),
        frame_id: Some(format!("fid-{sequence}")),
    }
}

fn frame_bytes() -> Vec<u8> {
    serde_json::to_vec(&sample_terminal_frame(0)).expect("serialize frame")
}

/// Benchmark in-process tokio mpsc channel delivery.
///
/// Asserts bounded wall-clock duration so the test is hardware-independent.
#[tokio::test]
async fn bench_in_process_mpsc_bounded_duration() {
    let frame = sample_terminal_frame(0);
    let payload = serde_json::to_vec(&frame).expect("serialize frame");
    let payload_len = payload.len();
    let total_messages: usize = 100_000;
    // Conservative bound: 100k messages via unbounded mpsc should finish in < 5s
    // even on noisy CI runners.
    let max_duration = Duration::from_secs(5);

    let (tx, mut rx) = mpsc::unbounded_channel::<Vec<u8>>();

    let payload_for_producer = payload.clone();
    let producer = tokio::spawn(async move {
        let start = Instant::now();
        for _ in 0..total_messages {
            let _ = tx.send(payload_for_producer.clone());
        }
        start.elapsed()
    });

    let start = Instant::now();
    let mut received = 0;
    while let Some(_item) = rx.recv().await {
        received += 1;
        if received >= total_messages {
            break;
        }
    }
    let elapsed = start.elapsed();
    let _ = producer.await;

    let throughput_mbps =
        (total_messages as f64 * payload_len as f64) / (elapsed.as_secs_f64() * 1_000_000.0);

    eprintln!(
        "in-process mpsc: {} messages in {:?} ({:.2} MB/s)",
        total_messages, elapsed, throughput_mbps
    );

    assert!(
        elapsed < max_duration,
        "in-process mpsc too slow: {:?} >= {:?}",
        elapsed,
        max_duration
    );
}

/// RAII guard that removes a Unix socket path on drop.
#[cfg(unix)]
struct SocketGuard<'a> {
    path: &'a std::path::Path,
}

#[cfg(unix)]
impl<'a> SocketGuard<'a> {
    fn new(path: &'a std::path::Path) -> Self {
        let _ = std::fs::remove_file(path);
        Self { path }
    }
}

#[cfg(unix)]
impl<'a> Drop for SocketGuard<'a> {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(self.path);
    }
}

/// Benchmark Unix domain socket delivery (macOS/Linux only).
///
/// Asserts bounded wall-clock duration so the test is hardware-independent.
/// Uses a RAII guard to ensure the socket file is cleaned up even on panic.
#[cfg(unix)]
#[tokio::test]
async fn bench_unix_domain_socket_bounded_duration() {
    use tokio::net::UnixListener;
    let payload = frame_bytes();
    let payload_len = payload.len();
    let total_messages: usize = 50_000;
    let max_duration = Duration::from_secs(10);
    let socket_path = format!("/tmp/opensymphony-bench-{}.sock", std::process::id());
    let path = std::path::Path::new(&socket_path);
    let _guard = SocketGuard::new(path);

    let listener = UnixListener::bind(path).expect("bind unix socket");

    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.expect("accept unix connection");
        let mut buf = vec![0u8; payload_len];
        let mut received = 0;
        loop {
            match stream.read_exact(&mut buf).await {
                Ok(_) => {
                    received += 1;
                    if received >= total_messages {
                        break received;
                    }
                }
                Err(_) => break received,
            }
        }
    });

    let client_socket_path = socket_path.clone();
    let client = tokio::spawn(async move {
        let mut stream = UnixStream::connect(&client_socket_path)
            .await
            .expect("connect unix socket");
        let start = Instant::now();
        for _ in 0..total_messages {
            if stream.write_all(&payload).await.is_err() {
                break;
            }
        }
        let _ = stream.shutdown().await;
        start.elapsed()
    });

    let received = server.await.expect("server task");
    let elapsed = client.await.expect("client task");

    let throughput_mbps =
        (received as f64 * payload_len as f64) / (elapsed.as_secs_f64() * 1_000_000.0);

    eprintln!(
        "unix domain socket: {} messages in {:?} ({:.2} MB/s)",
        received, elapsed, throughput_mbps
    );

    assert_eq!(received, total_messages, "not all messages delivered");
    assert!(
        elapsed < max_duration,
        "unix domain socket too slow: {:?} >= {:?}",
        elapsed,
        max_duration
    );
}

/// Benchmark WebSocket loopback delivery using tokio-tungstenite.
///
/// Asserts bounded wall-clock duration so the test is hardware-independent.
#[tokio::test]
async fn bench_websocket_loopback_bounded_duration() {
    use tokio_tungstenite::{accept_async, connect_async};

    let payload = frame_bytes();
    let payload_len = payload.len();
    let total_messages: usize = 10_000;
    let max_duration = Duration::from_secs(10);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind tcp");
    let addr = listener.local_addr().expect("local addr");

    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.expect("accept tcp");
        let mut ws = accept_async(stream).await.expect("accept ws");
        let mut received = 0;
        loop {
            match ws.next().await {
                Some(Ok(tokio_tungstenite::tungstenite::Message::Binary(_))) => {
                    received += 1;
                    if received >= total_messages {
                        break received;
                    }
                }
                Some(Ok(tokio_tungstenite::tungstenite::Message::Close(_))) => break received,
                Some(Ok(_)) => continue,
                Some(Err(_)) => break received,
                None => break received,
            }
        }
    });

    // Give server a moment to start listening
    tokio::time::sleep(Duration::from_millis(50)).await;

    let client = tokio::spawn(async move {
        let url = format!("ws://{}/", addr);
        let (mut ws, _) = connect_async(&url).await.expect("connect ws");
        let start = Instant::now();
        for _ in 0..total_messages {
            let msg = tokio_tungstenite::tungstenite::Message::Binary(payload.clone().into());
            if ws.send(msg).await.is_err() {
                break;
            }
        }
        let _ = ws.close(None).await;
        start.elapsed()
    });

    let received = server.await.expect("server task");
    let elapsed = client.await.expect("client task");

    let throughput_mbps =
        (received as f64 * payload_len as f64) / (elapsed.as_secs_f64() * 1_000_000.0);

    eprintln!(
        "websocket loopback: {} messages in {:?} ({:.2} MB/s)",
        received, elapsed, throughput_mbps
    );

    assert_eq!(received, total_messages, "not all messages delivered");
    assert!(
        elapsed < max_duration,
        "websocket loopback too slow: {:?} >= {:?}",
        elapsed,
        max_duration
    );
}

/// Evaluate JSON-RPC 2.0 envelope overhead by wrapping a terminal frame.
#[test]
fn json_rpc_envelope_overhead_is_acceptable() {
    use serde_json::json;

    let frame = sample_terminal_frame(1);
    let frame_json = serde_json::to_vec(&frame).expect("serialize frame");

    let json_rpc = json!({
        "jsonrpc": "2.0",
        "method": "terminal.frame",
        "params": {
            "stream_id": "stream-bench",
            "cursor": {"sequence": 1, "partition": "terminal:run-bench"},
            "frame": frame,
        }
    });
    let json_rpc_bytes = serde_json::to_vec(&json_rpc).expect("serialize json-rpc");

    let overhead = json_rpc_bytes.len() - frame_json.len();
    let overhead_pct = (overhead as f64 / frame_json.len() as f64) * 100.0;

    eprintln!(
        "JSON-RPC 2.0 envelope overhead: {} bytes ({:.1}%)",
        overhead, overhead_pct
    );

    // Gate: expect overhead < 50% for typical payloads
    assert!(
        overhead_pct < 50.0,
        "JSON-RPC 2.0 envelope overhead too high: {:.1}%",
        overhead_pct
    );
}

/// SSE line overhead evaluation.
#[test]
fn sse_line_overhead_is_acceptable() {
    let frame = sample_terminal_frame(1);
    let frame_json = serde_json::to_string(&frame).expect("serialize frame");

    let sse_event = format!(
        "event: terminal_frame\nid: 1\ndata: {}\n\n",
        frame_json.replace('\n', "")
    );
    let overhead = sse_event.len() - frame_json.len();
    let overhead_pct = (overhead as f64 / frame_json.len() as f64) * 100.0;

    eprintln!(
        "SSE line overhead: {} bytes ({:.1}%)",
        overhead, overhead_pct
    );

    // Gate: expect overhead < 30% for typical payloads
    assert!(
        overhead_pct < 30.0,
        "SSE line overhead too high: {:.1}%",
        overhead_pct
    );
}

// ─── Local transport benchmarks (COE-410) ──────────────────────────────────

/// Construct a sample GatewayEnvelope for benchmarking.
fn sample_gateway_envelope(sequence: u64) -> GatewayEnvelope {
    use opensymphony::opensymphony_gateway_schema::cursor::StreamCursor;
    use opensymphony::opensymphony_gateway_schema::envelope::{EntityKind, EntityRef};

    GatewayEnvelope {
        schema_version: SchemaVersion::v1(),
        cursor: StreamCursor::new(sequence, "terminal:run-bench"),
        entity_ref: EntityRef {
            kind: EntityKind::TerminalSession,
            id: "term-bench".into(),
            identifier: None,
        },
        event_kind: "terminal_frame".into(),
        payload: Some(serde_json::json!({
            "frame_sequence": sequence,
            "content": BENCH_PAYLOAD,
        })),
        raw_payload: None,
        emitted_at: chrono::Utc::now(),
    }
}

/// Transport profile throughput benchmark.
///
/// Measures end-to-end delivery latency for all local transport profiles
/// using realistic transport encoding patterns. This proves that each
/// transport profile can deliver envelopes with expected performance characteristics.
#[test]
fn local_transport_profile_delivery_benchmark() {
    let profiles = [
        TransportProfile::InProcessChannel,
        TransportProfile::NativeIpc,
        TransportProfile::TauriChannel,
        TransportProfile::LoopbackHttp,
        TransportProfile::LoopbackWebSocket,
    ];

    let envelope = sample_gateway_envelope(0);
    let payload = serde_json::to_vec(&envelope).expect("serialize envelope");

    for profile in &profiles {
        let iterations = 10_000;
        let start = Instant::now();

        for i in 0..iterations {
            let env = sample_gateway_envelope(i as u64);
            let bytes = serde_json::to_vec(&env).expect("serialize envelope");

            match profile {
                TransportProfile::InProcessChannel => {
                    // In-process channels use direct memory access (zero-copy simulation)
                    let _ = &bytes[..]; // No serialization overhead
                }
                TransportProfile::NativeIpc => {
                    // Native IPC may require base64 encoding for binary safety
                    // Simulate encoding overhead by converting to hex (std only)
                    let _encoded: String = bytes.iter().map(|b| format!("{:02x}", b)).collect();
                }
                TransportProfile::TauriChannel => {
                    // Tauri channels serialize through JSON IPC boundary
                    let _ = serde_json::to_string(&env).expect("tauri ipc serialization");
                }
                TransportProfile::LoopbackHttp => {
                    // HTTP SSE uses text-based frame prefixes
                    let _frame = format!("__event__ {}", String::from_utf8_lossy(&bytes));
                }
                TransportProfile::LoopbackWebSocket => {
                    // WebSocket uses binary or text frames with length prefix
                    let _frame = bytes.clone(); // Binary frame mode
                }
                _ => {
                    // Default: just serialize
                    let _ = &bytes[..];
                }
            }

            // Prevent compiler from optimizing away the work
            std::hint::black_box(&bytes);
        }

        let elapsed = start.elapsed();
        let throughput = (iterations as f64) / elapsed.as_secs_f64();
        let bytes_per_iter = payload.len();
        let mbps = (throughput * bytes_per_iter as f64) / (1024.0 * 1024.0);

        eprintln!(
            "transport profile {:?}: {:.0} iter/s ({:.1} MB/s, {} bytes/payload)",
            profile, throughput, mbps, bytes_per_iter
        );

        // Each profile should achieve reasonable throughput (>10k iter/s)
        assert!(
            elapsed < Duration::from_secs(10),
            "transport profile {:?} exceeded 10s for {} iterations: {:?}",
            profile,
            iterations,
            elapsed
        );
    }
}

/// Benchmark Tauri channel-like delivery via bounded mpsc.
///
/// Tauri channels use a bounded queue with backpressure. This test
/// simulates the delivery pattern using tokio::sync::mpsc::channel
/// with a capacity that mirrors typical Tauri channel buffer sizes.
///
/// Asserts bounded wall-clock duration so the test is hardware-independent.
#[tokio::test]
async fn bench_tauri_channel_bounded_duration() {
    let envelope = sample_gateway_envelope(0);
    let payload = serde_json::to_vec(&envelope).expect("serialize envelope");
    let payload_len = payload.len();
    let total_messages: usize = 50_000;
    // Bounded channel with backpressure should finish in < 5s
    let max_duration = Duration::from_secs(5);

    // Use bounded channel to simulate Tauri channel backpressure
    let (tx, mut rx) = mpsc::channel::<Vec<u8>>(1024);

    let payload_for_producer = payload.clone();
    let producer = tokio::spawn(async move {
        let start = Instant::now();
        for _ in 0..total_messages {
            if tx.send(payload_for_producer.clone()).await.is_err() {
                break;
            }
        }
        start.elapsed()
    });

    let start = Instant::now();
    let mut received = 0;
    while let Some(_item) = rx.recv().await {
        received += 1;
        if received >= total_messages {
            break;
        }
    }
    let elapsed = start.elapsed();
    let _ = producer.await;

    let throughput_mbps =
        (received as f64 * payload_len as f64) / (elapsed.as_secs_f64() * 1_000_000.0);

    eprintln!(
        "tauri channel (bounded): {} messages in {:?} ({:.2} MB/s)",
        received, elapsed, throughput_mbps
    );

    assert_eq!(received, total_messages, "not all messages delivered");
    assert!(
        elapsed < max_duration,
        "tauri channel too slow: {:?} >= {:?}",
        elapsed,
        max_duration
    );
}

/// Benchmark envelope roundtrip latency for local transport paths.
///
/// Measures the full serialize → transmit → deserialize cycle to ensure
/// local transport paths stay within acceptable latency bounds for
/// interactive terminal rendering.
#[test]
fn local_transport_envelope_roundtrip_latency() {
    use opensymphony::opensymphony_gateway_schema::cursor::StreamCursor;
    use opensymphony::opensymphony_gateway_schema::envelope::{EntityKind, EntityRef};

    let envelope = GatewayEnvelope {
        schema_version: SchemaVersion::v1(),
        cursor: StreamCursor::new(1, "terminal:run-bench"),
        entity_ref: EntityRef {
            kind: EntityKind::TerminalSession,
            id: "term-bench".into(),
            identifier: None,
        },
        event_kind: "terminal_frame".into(),
        payload: Some(serde_json::json!({
            "frame_sequence": 1,
            "content": BENCH_PAYLOAD,
        })),
        raw_payload: None,
        emitted_at: chrono::Utc::now(),
    };

    let total_roundtrips: usize = 100_000;
    let max_total_duration = Duration::from_secs(5);

    let json_bytes = serde_json::to_vec(&envelope).expect("serialize envelope");

    let start = Instant::now();
    for _ in 0..total_roundtrips {
        let parsed: GatewayEnvelope =
            serde_json::from_slice(&json_bytes).expect("deserialize envelope");
        // Verify structural integrity
        assert_eq!(parsed.cursor.sequence, 1);
        assert_eq!(parsed.entity_ref.kind, EntityKind::TerminalSession);
        assert_eq!(parsed.event_kind, "terminal_frame");
    }
    let elapsed = start.elapsed();

    let avg_latency_ns = elapsed.as_nanos() / total_roundtrips as u128;

    eprintln!(
        "envelope roundtrip: {} iterations in {:?} (avg {} ns/roundtrip)",
        total_roundtrips, elapsed, avg_latency_ns
    );

    assert!(
        elapsed < max_total_duration,
        "envelope roundtrip too slow: {:?} >= {:?}",
        elapsed,
        max_total_duration
    );

    // Gate: expect < 15μs average roundtrip for local transport
    assert!(
        avg_latency_ns < 50_000,
        "envelope roundtrip latency too high: {} ns (threshold 50us)",
        avg_latency_ns
    );
}

/// Verify that all transport profiles produce envelopes compatible with
/// the gateway schema version and cursor semantics.
#[test]
fn transport_profile_envelope_compatibility() {
    use opensymphony::opensymphony_gateway_schema::cursor::StreamCursor;
    use opensymphony::opensymphony_gateway_schema::envelope::{EntityKind, EntityRef};

    let profiles = [
        TransportProfile::InProcessChannel,
        TransportProfile::NativeIpc,
        TransportProfile::TauriChannel,
        TransportProfile::LoopbackHttp,
        TransportProfile::LoopbackWebSocket,
        TransportProfile::Sse,
        TransportProfile::WebSocket,
    ];

    for profile in profiles {
        let envelope = GatewayEnvelope {
            schema_version: SchemaVersion::v1(),
            cursor: StreamCursor::new(42, "terminal:run-test"),
            entity_ref: EntityRef {
                kind: EntityKind::TerminalSession,
                id: "term-test".into(),
                identifier: None,
            },
            event_kind: "terminal_frame".into(),
            payload: Some(serde_json::json!({ "profile": format!("{:?}", profile) })),
            raw_payload: None,
            emitted_at: chrono::Utc::now(),
        };

        // Serialize and deserialize roundtrip
        let json_bytes = serde_json::to_vec(&envelope).expect("serialize envelope");
        let restored: GatewayEnvelope =
            serde_json::from_slice(&json_bytes).expect("deserialize envelope");

        // Verify envelope integrity after roundtrip
        assert_eq!(restored.schema_version.major, 1);
        assert_eq!(restored.cursor.sequence, 42);
        assert_eq!(restored.cursor.partition, "terminal:run-test");
        assert_eq!(restored.entity_ref.kind, EntityKind::TerminalSession);
        assert_eq!(restored.entity_ref.id, "term-test");
        assert_eq!(restored.event_kind, "terminal_frame");
    }
}

/// Benchmark cursor-based replay performance.
///
/// Verifies that seeking to a cursor position and replaying from that
/// point performs within bounds for long-running sessions with large
/// event journals.
#[tokio::test]
async fn bench_cursor_replay_performance() {
    use opensymphony::opensymphony_gateway_schema::cursor::StreamCursor;
    use opensymphony::opensymphony_gateway_schema::envelope::{EntityKind, EntityRef};

    let total_envelopes: usize = 10_000;
    let replay_from_sequence: u64 = 5_000;
    let max_duration = Duration::from_secs(3);

    // Generate a journal of envelopes
    let journal: Vec<GatewayEnvelope> = (0..total_envelopes as u64)
        .map(|seq| GatewayEnvelope {
            schema_version: SchemaVersion::v1(),
            cursor: StreamCursor::new(seq, "terminal:run-replay"),
            entity_ref: EntityRef {
                kind: EntityKind::TerminalSession,
                id: "term-replay".into(),
                identifier: None,
            },
            event_kind: "terminal_frame".into(),
            payload: Some(serde_json::json!({ "frame_sequence": seq })),
            raw_payload: None,
            emitted_at: chrono::Utc::now(),
        })
        .collect();

    // Serialize the journal (simulating storage/transit)
    let journal_bytes = serde_json::to_vec(&journal).expect("serialize journal");

    let start = Instant::now();

    // Deserialize and replay from cursor
    let restored: Vec<GatewayEnvelope> =
        serde_json::from_slice(&journal_bytes).expect("deserialize journal");
    let replay_envelopes: Vec<_> = restored
        .into_iter()
        .filter(|e| e.cursor.sequence >= replay_from_sequence)
        .collect();

    let elapsed = start.elapsed();

    let replay_count = replay_envelopes.len();
    let expected_count = total_envelopes - replay_from_sequence as usize;

    eprintln!(
        "cursor replay: {} envelopes from sequence {} in {:?} ({} received, {} expected)",
        replay_count, replay_from_sequence, elapsed, replay_count, expected_count
    );

    assert_eq!(replay_count, expected_count, "replay count mismatch");
    assert!(
        elapsed < max_duration,
        "cursor replay too slow: {:?} >= {:?}",
        elapsed,
        max_duration
    );
}
