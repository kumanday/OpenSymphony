# OpenSymphony ACP Debugging Integration for Zed

Date: 2026-06-14

## Purpose

Define the implementation plan for exposing OpenSymphony issue debug sessions through Zed's ACP external-agent interface while preserving the existing OpenHands conversation model and current OpenSymphony workspace structure.

The goal is to make Zed the code, diff, and manual intervention surface for a selected issue workspace, while OpenSymphony remains the authority for resolving issue workspaces, conversation manifests, OpenHands stores, runtime streams, and debug turns.

## Current authoritative state

This spec is grounded in the current OpenSymphony repository behavior and the observed local runtime layout.

### User-level runtime layout

OpenSymphony-managed local state currently lives under the user-level directory:

```text
~/.opensymphony/
  openhands-server/
  quarantine/
  workspaces/
```

Managed OpenHands tooling lives under:

```text
~/.opensymphony/openhands-server/
```

Existing local installations may have a flat OpenHands conversation store:

```text
~/.opensymphony/openhands-server/workspace/conversations/<compact-conversation-uuid>/
```

The source code names this flat store the `Legacy` store. It remains a supported lookup source.

The source code also defines repo-scoped managed stores:

```text
~/.opensymphony/openhands-server/workspace/conversations/repos/<repo-key>/active/
~/.opensymphony/openhands-server/workspace/conversations/repos/<repo-key>/archived/
```

The repo-scoped stores may be absent in existing installations until OpenSymphony creates or migrates them. They are part of the managed-store model, not a universal assumption about every local machine.

### Issue workspace layout

Each issue has a deterministic workspace under the configured workspace root, commonly:

```text
~/.opensymphony/workspaces/<issue-key>/
```

The issue workspace contains workspace-local OpenSymphony metadata:

```text
~/.opensymphony/workspaces/<issue-key>/.opensymphony/
  issue.json
  conversation.json
  openhands/
    create-conversation-request.json
  logs/
  generated/
  prompts/
  runs/
```

The important manifest for ACP debug attachment is:

```text
~/.opensymphony/workspaces/<issue-key>/.opensymphony/conversation.json
```

That manifest contains the durable OpenHands conversation identity and launch context, including:

```text
issue_id
identifier
conversation_id
server_base_url
persistence_dir
created_at
last_attached_at
fresh_conversation
reset_reason
runtime_contract_version
```

### Existing debug behavior

`opensymphony debug <issue-id>` currently:

1. Resolves runtime configuration.
2. Builds a `WorkspaceManager`.
3. Finds the issue workspace by issue reference.
4. Loads the workspace-local issue manifest.
5. Loads the workspace-local `.opensymphony/conversation.json` manifest.
6. Parses the OpenHands `conversation_id`.
7. Resolves the OpenHands conversation store through active, archived, or legacy paths.
8. Builds or reuses an OpenHands client and local server supervisor.
9. Attaches a runtime event stream to the conversation.
10. Enters an interactive terminal debug loop.

The ACP implementation should reuse this resolution and attachment behavior. It should avoid introducing a new debug manifest or a parallel runtime state model.

## Desired UX

### Zed as the IDE debug surface

The Tauri app should expose a debug action for an issue. For example:

```text
Debug in Zed
```

The action should:

1. Resolve the selected issue key to its exact OpenSymphony issue workspace path.
2. Launch Zed on that exact workspace path.
3. Keep the rich OpenSymphony orchestration and visualization UI in Tauri.
4. Let the operator start the configured OpenSymphony Debug external agent in Zed.

Expected launch shape:

```bash
zed -n ~/.opensymphony/workspaces/COE-370
```

At MVP scope, the operator starts the OpenSymphony Debug external agent from Zed's Agent Panel, Threads Sidebar, or a user-configured Zed keybinding. The spec does not assume a documented Zed CLI or URI API for opening a workspace and auto-starting a particular external-agent thread.

### Zed ACP process model

Zed runs a statically configured ACP external-agent command. The command is independent of the specific issue key.

Recommended one-time Zed configuration:

```json
{
  "agent_servers": {
    "opensymphony-debug": {
      "type": "custom",
      "command": "opensymphony",
      "args": ["debug", "--acp-stdio"],
      "env": {}
    }
  }
}
```

When the operator starts the OpenSymphony Debug external agent in Zed, Zed spawns:

```bash
opensymphony debug --acp-stdio
```

Zed then sends ACP JSON-RPC messages over the subprocess stdin/stdout stream. The issue workspace is communicated through the ACP `session/new` request, not through the process command line.

Expected ACP request shape:

```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "method": "session/new",
  "params": {
    "cwd": "/Users/magos/.opensymphony/workspaces/COE-370",
    "mcpServers": []
  }
}
```

The ACP server mode must treat `params.cwd` as the authoritative workspace selection input.

## Command surface

Keep ACP debugging inside the existing `debug` command family.

### `opensymphony debug <issue-key>`

Default user-facing debug entrypoint.

Initial behavior:

1. Resolve `<issue-key>` to the exact issue workspace.
2. Launch or prepare the preferred IDE debug experience, initially Zed.
3. Print concise instructions if the IDE cannot be launched or if the operator must start the Zed external agent manually.

This command should become the primary operator-facing debug UX.

### `opensymphony debug <issue-key> --cli`

Compatibility mode for the existing terminal interactive debug loop.

This path should preserve current behavior as closely as possible:

1. Resolve issue workspace.
2. Load `.opensymphony/conversation.json`.
3. Attach to OpenHands.
4. Print recent history.
5. Accept terminal prompts.
6. Send user messages and run the OpenHands conversation.

### `opensymphony debug --acp-stdio`

Noninteractive ACP server mode for Zed and other ACP clients.

Behavior:

1. Start an ACP JSON-RPC server over stdio.
2. Accept `session/new` with a `cwd` parameter.
3. Resolve `cwd` as an exact OpenSymphony issue workspace root.
4. Attach to the workspace's OpenHands conversation.
5. Serve ACP prompts, events, and session teardown.

Rules:

1. No issue key is required in this mode.
2. Human-readable protocol output must not be written to stdout.
3. The process exits when the ACP client closes the stream or terminates the session.
4. This mode should be hidden from ordinary help text unless there is already a convention for advanced flags.

## Workspace selection policy

The ACP handler must use strict workspace selection.

### Valid ACP `cwd`

A valid `cwd` is the exact issue workspace root:

```text
~/.opensymphony/workspaces/<issue-key>
```

The following files must exist:

```text
cwd/.opensymphony/issue.json
cwd/.opensymphony/conversation.json
```

`conversation.json` is the source for `conversation_id` and runtime attach context.

### Invalid ACP `cwd`

Reject the session when `cwd` is any of the following:

```text
~/.opensymphony/workspaces/
~/.opensymphony/workspaces/<issue-key>/some/nested/path
/path/to/target-repo
~/.opensymphony/openhands-server/workspace/conversations/<uuid>
~/.opensymphony/openhands-server/workspace/conversations/repos/<repo-key>/active/<uuid>
~/.opensymphony/openhands-server/workspace/conversations/repos/<repo-key>/archived/<uuid>
```

The ACP handler should return an actionable error message such as:

```text
OpenSymphony Debug must be started from an exact issue workspace root.
Open ~/.opensymphony/workspaces/COE-370 in Zed, then start the OpenSymphony Debug agent again.
```

### No fuzzy workspace resolution

The ACP handler should not walk upward to find `.opensymphony/conversation.json`.

The ACP handler should not infer an issue workspace from an OpenHands conversation-store directory.

The ACP handler should not scan all workspaces to guess a conversation for the current project.

## OpenHands conversation store resolution

Workspace resolution and OpenHands store resolution are separate steps.

Step 1 resolves the OpenSymphony issue workspace:

```text
ACP session/new.cwd
  → cwd/.opensymphony/conversation.json
  → conversation_id
```

Step 2 resolves the durable OpenHands conversation storage:

```text
conversation_id
  → repo-scoped active store
  → repo-scoped archived store
  → legacy flat store
```

This should reuse the existing `OpenHandsConversationStorePaths` logic and the existing debug preparation behavior.

The legacy flat store must remain accepted as a valid OpenHands conversation source:

```text
~/.opensymphony/openhands-server/workspace/conversations/<compact-conversation-uuid>/
```

The ACP attachment should use the same attach and rehydrate behavior as the existing terminal debug path.

## ACP session semantics

ACP session state is an attachment to a durable OpenHands conversation.

Internal mapping:

```text
ACP session id
  → issue key
  → issue workspace path
  → conversation manifest
  → OpenHands conversation_id
  → RuntimeEventStream
  → optional LocalServerSupervisor ownership
```

The ACP session id should be generated by OpenSymphony. It can include the issue key for readability, but it should remain an ACP attachment id rather than a raw OpenHands conversation id.

Recommended shape:

```text
opensymphony-debug:<issue-key>:<uuid>
```

## ACP method behavior

### Initialize

Expose OpenSymphony as a debug-capable ACP external agent.

The agent name should be stable, for example:

```text
OpenSymphony Debug
```

The initial implementation should advertise only the capabilities it implements.

### `session/new`

Input:

```text
params.cwd
```

Behavior:

1. Validate that `cwd` is an exact issue workspace root.
2. Load `cwd/.opensymphony/conversation.json`.
3. Load `cwd/.opensymphony/issue.json` if needed for issue state and display.
4. Resolve runtime config from the target repo context using the same approach as current debug behavior.
5. Resolve OpenHands store kind through existing active, archived, and legacy logic.
6. Build or reuse the OpenHands client and local server supervisor.
7. Attach a `RuntimeEventStream` to the existing OpenHands conversation.
8. If the conversation has a turn in progress, stream status and wait until the turn is safe for user input, matching current terminal debug behavior.
9. Return an ACP session id.
10. Send an initial assistant message summarizing the attached issue, workspace, conversation id, and current execution status.

Initial message example:

```text
Attached to OpenSymphony issue COE-370.
Workspace: ~/.opensymphony/workspaces/COE-370
Conversation: 1f3e...
Status: idle

You can review code and diffs in this Zed workspace. Send a message here to continue the existing OpenHands conversation.
```

### `session/prompt`

Input:

```text
ACP session id
user message
```

Behavior:

1. Verify the ACP session is attached.
2. If an OpenHands turn is already in progress, wait for the current turn to stop using the existing debug wait behavior.
3. Send the user message to the OpenHands conversation through `OpenHandsClient::send_message`.
4. Invoke `OpenHandsClient::run_conversation`.
5. Stream normalized assistant, action, and observation events back to Zed.
6. End the ACP prompt response when the OpenHands turn reaches a terminal or idle state.

The existing terminal debug function already performs the core sequence:

```text
send_message(conversation_id, user_text)
run_conversation(conversation_id)
wait_for_turn_terminal(...)
```

The ACP implementation should extract this into reusable debug-turn logic instead of duplicating protocol-specific code paths.

### `session/close`

Behavior:

1. Close the runtime event stream.
2. Release any ACP session subscriptions.
3. Drop or stop a local supervisor only when this ACP process owns the supervisor.
4. Leave the durable OpenHands conversation intact.
5. Leave workspace files intact.
6. Leave `.opensymphony/conversation.json` intact.
7. Leave repo memory intact.

Closing an ACP session is a detach operation for this integration.

### Optional methods

The MVP does not require `session/list`, `session/load`, or `session/resume`.

Those methods can be added after the cwd-based attach path is stable.

A future `session/list` could expose known issue workspaces as importable sessions, but it should not be part of the first implementation slice.

## Event mapping

OpenSymphony should map existing normalized OpenHands runtime events into ACP updates that Zed can display in the agent thread.

Recommended role mapping:

```text
OpenHands user message       → ACP user message echo or prompt boundary
OpenHands assistant text     → ACP assistant message/update
OpenHands action/tool event  → ACP action or structured status message
OpenHands observation        → ACP observation or structured status message
Execution status changes     → ACP progress/status update
Errors                       → ACP error update with concise recovery guidance
```

The event stream should preserve useful debug context:

```text
event id
timestamp
role or event kind
summary text
raw details when safe and useful
execution status
```

Sensitive values should follow existing secret-redaction policy.

## Refactor plan

The current terminal debug implementation should be split into reusable layers.

### `debug_session` core layer

Extract reusable primitives from the current debug code:

```text
resolve_debug_runtime_config(args or cwd)
resolve_issue_workspace(issue key)
load_issue_manifest(workspace)
load_conversation_manifest(workspace)
prepare_debug_conversation_store(runtime, conversation_id, issue_manifest)
build_debug_client(runtime, store_kind)
attach_or_rehydrate_stream(...)
wait_for_turn_to_stop(...)
run_debug_turn(...)
```

### `DebugAttachment`

Introduce an internal struct for active debug attachment state:

```rust
struct DebugAttachment {
    session_id: String,
    issue_key: String,
    workspace_path: PathBuf,
    conversation_id: Uuid,
    client: OpenHandsClient,
    stream: RuntimeEventStream,
    supervisor: Option<LocalServerSupervisor>,
}
```

This struct should support:

```text
send_prompt(...)
stream_events(...)
close(...)
status(...)
```

### CLI terminal layer

The `--cli` path should use `DebugAttachment` and then enter the existing terminal read/eval loop.

### ACP stdio layer

The `--acp-stdio` path should use `DebugAttachment` and serve JSON-RPC over stdio.

It should contain only protocol adaptation logic:

```text
ACP request parsing
ACP response serialization
ACP session lifecycle
ACP update emission
mapping DebugAttachment events to ACP messages
```

## Tauri integration

The Tauri app should call an OpenSymphony API or internal command to resolve an issue key to the exact issue workspace path.

Inputs:

```text
issue key
preferred editor = zed
```

Output:

```text
workspace path
launch command
operator instruction text, when needed
```

For Zed:

```bash
zed -n <workspace-path>
```

The app should not create per-issue Zed agent configurations.

The app should not write additional workspace debug manifests.

The app may provide a one-time setup UX for the static Zed external-agent configuration.

## Zed integration

Zed should have one static OpenSymphony Debug external-agent configuration.

The configured command should be:

```bash
opensymphony debug --acp-stdio
```

The operator opens the issue workspace in Zed and starts the OpenSymphony Debug agent.

The ACP `cwd` supplied by Zed is expected to equal the opened project root. The OpenSymphony handler validates this by requiring `cwd/.opensymphony/conversation.json` and `cwd/.opensymphony/issue.json`.

## Concurrency policy

MVP policy:

```text
one active ACP debug session per spawned opensymphony debug --acp-stdio process
```

If Zed starts multiple external-agent processes, each process may attach to one workspace.

If the same process receives a second `session/new` while a session is active, return an ACP error:

```text
This OpenSymphony Debug ACP process already has an active session. Close the current debug thread before starting another one.
```

This policy can be relaxed later if Zed UX and ACP process lifecycle behavior make multiplexing valuable.

## Failure behavior

### Invalid cwd

Return a user-facing ACP error with the expected path shape.

### Missing conversation manifest

Return:

```text
No OpenSymphony conversation manifest was found at cwd/.opensymphony/conversation.json.
Open an exact OpenSymphony issue workspace in Zed, then start OpenSymphony Debug again.
```

### Invalid conversation id

Return a manifest validation error naming the manifest path and invalid field.

### OpenHands conversation missing

Use the existing attach-or-rehydrate behavior from terminal debug.

If the existing code would rehydrate, ACP should rehydrate.

If the existing code would report an archived-store mismatch or unavailable conversation, ACP should report the same condition in a concise agent-thread message.

### Active turn already running

Mirror current terminal debug behavior:

1. Inform the user that a turn is already in progress.
2. Wait until the turn stops.
3. Continue accepting input if the wait succeeds.
4. Report timeout without detaching if the wait times out.

### Existing OpenHands server on same port with different store

Surface the same operational guidance as current debug behavior:

```text
Stop the existing OpenHands server or free the port, then retry the debug session.
```

## Safety and state boundaries

The ACP debug integration must preserve the existing ownership model.

OpenSymphony owns:

```text
issue workspace resolution
conversation manifest interpretation
OpenHands store selection
runtime stream attachment
conversation turn execution
local server supervision when applicable
```

Zed owns:

```text
code inspection
manual file edits
diff review
agent thread UI
ACP client process management
```

Tauri owns:

```text
orchestration cockpit
visual trace UI
debug action routing
Zed launch action
one-time integration setup UX, if implemented
```

The ACP adapter must not:

```text
create .opensymphony/debug.json
write per-issue Zed agent configs
delete OpenHands conversations during session close
delete issue workspaces during session close
alter repo memory during session close
infer workspaces from nested paths or parent paths
```

## Test plan

### Unit tests

Add tests for:

```text
valid cwd with issue.json and conversation.json
invalid parent workspace root
invalid nested subdirectory
invalid target repo root
invalid OpenHands conversation store path
conversation manifest parsing
invalid conversation id
legacy flat store lookup
active store lookup
archived store lookup
single active ACP session enforcement
```

### Integration tests

Add an integration test using the existing fake OpenHands server and a minimal ACP stdio client harness.

Test flow:

```text
create fixture issue workspace
write .opensymphony/issue.json
write .opensymphony/conversation.json
create fake OpenHands conversation state
spawn opensymphony debug --acp-stdio
send initialize
send session/new with exact cwd
assert attached response
send session/prompt
assert OpenHands send_message and run_conversation were called
assert assistant/event updates are returned
send session/close
assert stream closes and workspace remains intact
```

### CLI regression tests

Add tests for:

```text
opensymphony debug COE-370 --cli routes to terminal debug path
opensymphony debug --acp-stdio starts ACP mode without requiring issue key
opensymphony debug --acp-stdio COE-370 is rejected or documented explicitly
opensymphony debug COE-370 resolves workspace for default IDE debug path
```

## Acceptance criteria

1. A single static Zed external-agent config can start OpenSymphony Debug.
2. Tauri can open Zed on an exact issue workspace.
3. Starting the OpenSymphony Debug agent in that Zed workspace attaches to the existing OpenHands conversation for that issue.
4. The ACP handler obtains the issue selection from `session/new.cwd`.
5. The ACP handler requires `cwd/.opensymphony/conversation.json` and `cwd/.opensymphony/issue.json`.
6. The legacy flat OpenHands conversation store remains supported.
7. Active and archived repo-scoped stores remain supported.
8. `opensymphony debug <issue-key> --cli` preserves the current terminal debug workflow.
9. `opensymphony debug --acp-stdio` writes only ACP protocol messages to stdout.
10. ACP session close detaches from the debug stream without deleting durable state.
11. No `.opensymphony/debug.json` or equivalent extra manifest is introduced.
12. No per-conversation Zed `agent_servers` entries are created.
13. One active ACP debug session per spawned ACP process is enforced for MVP.
14. Invalid cwd produces a concise, actionable error.

## Implementation slices

### Slice 1: Debug core refactor

Extract reusable resolution, attach, and turn execution logic from the current terminal debug command.

Deliverables:

```text
DebugAttachment core type
shared attach path
shared run-turn path
existing CLI behavior preserved behind --cli
```

### Slice 2: ACP stdio server mode

Add `opensymphony debug --acp-stdio`.

Deliverables:

```text
ACP initialize
ACP session/new with strict cwd validation
ACP session/prompt
ACP session/close
single-session enforcement
```

### Slice 3: Zed static integration guidance

Add documentation or app onboarding content for the one-time Zed external-agent config.

Deliverables:

```text
Zed settings snippet
usage instructions
failure guidance for invalid cwd
```

### Slice 4: Tauri debug launch

Wire the Tauri issue debug button to resolve and open the exact issue workspace in Zed.

Deliverables:

```text
resolve issue key to workspace path
launch zed -n <workspace-path>
show instruction to start OpenSymphony Debug agent inside Zed
```

### Slice 5: Default debug UX transition

Change the default `opensymphony debug <issue-key>` behavior to the IDE-oriented flow after slices 1 through 4 are stable.

Deliverables:

```text
plain debug opens/prepares IDE debug UX
--cli preserves terminal behavior
clear fallback messaging when Zed or Tauri is unavailable
```

## Source grounding

The following current source locations are the grounding points for this spec:

```text
crates/opensymphony-workspace/src/models.rs
  WorkspaceHandle metadata paths
  conversation_manifest_path()
  ConversationManifest fields

crates/opensymphony-openhands/src/conversation_store.rs
  OPENHANDS_CONVERSATIONS_PATH_ENV
  ConversationStoreKind: Active, Archived, Legacy
  OpenHandsConversationStorePaths
  active, archived, legacy store layout
  locate_conversation lookup order
  compact/raw UUID directory handling

crates/opensymphony-cli/src/debug_session.rs
  current debug command args
  runtime config resolution
  workspace lookup
  conversation manifest loading
  OpenHands store preparation
  debug client construction
  attach_or_rehydrate_stream
  interactive debug loop
  send_message plus run_conversation debug turn behavior
```
