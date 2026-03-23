---
name: custom-codereview-guide
description: |
  Repository-specific code review guidance for OpenSymphony.
  This skill supplements the default OpenHands review skill with project-specific
  rules for reviewing Rust code in an orchestrator/agent system.
---

# Custom Code Review Guide for OpenSymphony

## Project Context

OpenSymphony is a Rust implementation of the Symphony service specification:
- **Orchestrator**: Claims issues, manages worker lifecycle, handles retries
- **Linear Adapter**: Reads from Linear, normalizes issues
- **OpenHands Runtime**: Adapts OpenHands agent-server for execution
- **Workspace Manager**: Per-issue workspace isolation

## Review Focus Areas

### 1. Async and Concurrency

**Watch for:**
- Holding locks across await points
- Unbounded channels without backpressure
- Missing cancellation token propagation
- Blocking operations in async contexts

**Good patterns:**
```rust
// Prefer structured concurrency
let (tx, rx) = tokio::sync::mpsc::channel(100); // bounded

// Cancel safety
let result = tokio::select! {
    biased;
    _ = cancellation_token.cancelled() => return Ok(()),
    result = work => result,
};
```

### 2. Error Handling

**Watch for:**
- `unwrap()` or `expect()` in production code
- Generic `anyhow` errors without context
- Silent error swallowing
- Missing error variants in enums

**Good patterns:**
```rust
#[derive(Debug, thiserror::Error)]
pub enum WorkspaceError {
    #[error("path not contained in workspace root: {path}")]
    PathEscape { path: String },
    #[error("hook timeout after {timeout_ms}ms")]
    HookTimeout { timeout_ms: u64 },
}

// Context-rich errors
workspace.ensure(issue).await
    .context("failed to prepare workspace for issue {issue_id}")?;
```

### 3. Workspace Safety

**Watch for:**
- Path traversal vulnerabilities
- Unsanitized identifiers in paths
- Missing containment checks
- Hook execution without timeouts

**Critical rule:**
Every workspace path must pass `containment_check(path, workspace_root)`.

### 4. WebSocket and Network Resilience

**Watch for:**
- Infinite reconnect loops without backoff
- Missing readiness barriers
- No event reconciliation after reconnect
- Ignoring WebSocket close codes

**Good patterns:**
```rust
// Bounded exponential backoff
let backoff = ExponentialBackoff {
    initial: Duration::from_millis(100),
    max: Duration::from_secs(30),
    multiplier: 2.0,
};

// Reconcile after reconnect
let missed_events = rest_client
    .search_events(conversation_id, last_seen_event_id)
    .await?;
```

### 5. State Machine Correctness

**Watch for:**
- State transitions that bypass validation
- Missing terminal state handling
- Race conditions in claim/release
- Inconsistent retry metadata

**Key invariants:**
- Only the orchestrator mutates scheduling state
- Workers report outcomes, never mutate state directly
- Terminal states are idempotent

### 6. Serialization and Forward Compatibility

**Watch for:**
- `serde(flatten)` without raw JSON fallback
- Missing `#[serde(default)]` on new fields
- Breaking changes to persisted formats

**Good patterns:**
```rust
#[derive(Deserialize)]
pub struct Event {
    pub id: String,
    pub event_type: String,
    // Known fields
    pub payload: Option<serde_json::Value>,
    // Unknown fields preserved
    #[serde(flatten)]
    pub extra: serde_json::Value,
}
```

### 7. Testing

**Watch for:**
- Tests that depend on external services
- Missing failure case coverage
- Tests that don't clean up resources
- Mock-heavy tests that don't test real code paths

**Good patterns:**
- Use `opensymphony-testkit` fake servers
- Test state transitions exhaustively
- Property-based tests for retry calculations
- Integration tests with real file system but fake network

## Security-Sensitive Areas

### Extra scrutiny required for:

1. **Path handling** (`opensymphony-workspace`)
   - Any path construction from user input
   - Hook execution paths
   - Workspace root containment

2. **Secret handling** (workflow and config)
   - LLM API keys in logs
   - GitHub tokens in error messages
   - Linear API key exposure

3. **Agent execution** (`opensymphony-openhands`)
   - Command injection through tool parameters
   - File system escape via symlinks
   - Resource exhaustion (infinite loops, large outputs)

4. **Network boundaries**
   - WebSocket message validation
   - REST response parsing
   - GraphQL query injection

## Performance Considerations

**Watch for:**
- Unbounded memory growth in event caches
- Synchronous IO in async paths
- Missing backpressure in streaming
- Inefficient polling loops

## Documentation Requirements

Code changes should update:
- `docs/architecture.md` for structural changes
- `docs/*.md` for behavior changes
- Inline comments for non-obvious invariants
- `AGENTS.md` for repo-specific knowledge

## Evidence Requirements

PRs should include:
- **Behavior changes**: Test output showing before/after
- **Performance changes**: Benchmarks or timing data
- **New features**: Usage examples
- **Bug fixes**: Reproduction case and fix verification
