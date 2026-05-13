# Subscription-Based OpenAI ChatGPT/Codex Authentication with `openhands agent-server`

## Purpose

This document explains how OpenAI ChatGPT/Codex subscription-based authentication can work when using `openhands agent-server`, especially for architectures that want to use OpenHands SDK and remote execution while relying on a user's ChatGPT Plus/Pro subscription instead of an OpenAI API key.

The core idea is that OpenHands SDK subscription login does not introduce a completely separate agent interface. It authenticates with OpenAI/ChatGPT OAuth, obtains OAuth credentials, and then constructs a normal OpenHands `LLM` object configured to speak to the ChatGPT Codex backend.

That means an `Agent` and `Conversation` can use the resulting `LLM` object in the usual OpenHands SDK flow, including with a remote `agent-server` workspace. The architecture decision is where OAuth happens, where refresh credentials are stored, and how much credential material is exposed to remote execution infrastructure.

---

## Core SDK Behavior

OpenHands SDK exposes subscription authentication through:

```python
from openhands.sdk import LLM

llm = LLM.subscription_login(
    vendor="openai",
    model="gpt-5.2-codex",
)
```

At a high level, this does the following:

1. Performs OpenAI/ChatGPT OAuth for subscription-based Codex access.
2. Stores OAuth credentials locally.
3. Refreshes credentials when needed.
4. Builds a normal OpenHands `LLM` object.
5. Configures that `LLM` object for the ChatGPT Codex backend rather than the standard OpenAI API endpoint.

The underlying `OpenAISubscriptionAuth.create_llm()` path creates an `LLM` roughly equivalent to:

```python
LLM(
    model=f"openai/{model}",
    base_url="https://chatgpt.com/backend-api/codex",
    api_key=oauth_access_token,
    extra_headers={
        "originator": "codex_cli_rs",
        "OpenAI-Beta": "responses=experimental",
        "User-Agent": "...",
        "chatgpt-account-id": "...",  # when available
    },
    litellm_extra_body={"store": False},
    temperature=None,
    max_output_tokens=None,
    stream=True,
)
```

It also marks the LLM internally as subscription-backed:

```python
llm._is_subscription = True
```

This matters because subscription mode needs slightly different request handling and message transformation from normal OpenAI API usage.

---

## Credential Storage

The SDK stores OAuth credentials under the OpenHands auth directory, currently:

```text
~/.openhands/auth/
```

For OpenAI, the expected stored credential file is conceptually:

```text
~/.openhands/auth/openai_oauth.json
```

The credential payload includes:

- vendor
- access token
- refresh token
- expiration timestamp

The SDK can reuse cached credentials and refresh expired access tokens using the stored refresh token.

This credential location is fine for single-user local development. It should not be treated as an acceptable default for hosted multi-tenant systems without an additional credential isolation and encryption layer.

---

## How This Works with `openhands agent-server`

The `agent-server` model separates the SDK client and the remote execution workspace.

The SDK shape remains roughly:

```python
agent = Agent(llm=llm, tools=[...])
conversation = Conversation(agent=agent, workspace=workspace)
conversation.send_message("...")
conversation.run()
```

The key difference is that `workspace` can be backed by `DockerWorkspace` or `APIRemoteWorkspace`, causing the SDK to communicate with an `openhands agent-server` instance over HTTP/WebSocket.

So the subscription-backed `LLM` can be used with `agent-server` because, after login, it is still just an SDK `LLM` object attached to an SDK `Agent`.

---

## Mode 1: Personal or Local Agent-Server Use

For a single-user local or self-hosted setup, this is the simplest workable approach.

### Flow

1. User runs `LLM.subscription_login(...)` in the same environment that will create the `LLM`.
2. SDK opens browser OAuth or uses cached credentials.
3. Credentials are stored in `~/.openhands/auth/`.
4. SDK creates the subscription-backed `LLM`.
5. The `Agent` uses that `LLM`.
6. The `Conversation` uses a local or remote agent-server workspace.

### Example

```python
from openhands.sdk import LLM, Agent, Conversation, Tool
from openhands.tools.file_editor import FileEditorTool
from openhands.tools.terminal import TerminalTool
from openhands.sdk.workspace import DockerWorkspace

llm = LLM.subscription_login(
    vendor="openai",
    model="gpt-5.2-codex",
)

agent = Agent(
    llm=llm,
    tools=[
        Tool(name=TerminalTool.name),
        Tool(name=FileEditorTool.name),
    ],
)

with DockerWorkspace(
    server_image="ghcr.io/openhands/agent-server:latest",
) as workspace:
    conversation = Conversation(agent=agent, workspace=workspace)
    conversation.send_message("Inspect this repository and propose fixes.")
    conversation.run()
```

### Operational Requirement

Persist the user's OpenHands auth directory across runs:

```text
~/.openhands/auth/
```

If the agent-server or SDK process runs in a container, mount this directory explicitly.

Example:

```bash
-v ~/.openhands/auth:/home/openhands/.openhands/auth
```

Adjust the target path based on the container user and home directory.

---

## Mode 2: Headless or Remote Agent-Server Use

For remote or headless execution, browser OAuth can be awkward because the server may not have a GUI browser or may not be reachable through localhost callbacks.

The SDK has device-code login support for OpenAI subscription auth. This is the better path for remote/headless use.

### Example

```python
from openhands.sdk import LLM

llm = LLM.subscription_login(
    vendor="openai",
    model="gpt-5.2-codex",
    auth_method="device_code",
    open_browser=False,
)
```

The device-code flow prints a verification URL and one-time code. The user opens the URL in a browser, signs in to ChatGPT, and enters the code. The remote process polls until the authorization is complete, then exchanges the result for OAuth tokens and stores them.

### Requirements

The process performing login must have:

- Network access to OpenAI/ChatGPT auth endpoints.
- A persistent auth directory.
- A way to show the device-code URL and code to the user.
- A secure filesystem boundary for `~/.openhands/auth/`.

This is suitable for personal remote servers or self-hosted setups.

It is not sufficient as-is for a multi-tenant hosted environment unless credentials are isolated per user.

---

## Mode 3: Hosted Multi-User Architecture

For hosted multi-user use, avoid a shared `~/.openhands/auth/` inside a common agent-server container.

Recommended architecture:

```text
User browser / desktop app
  -> ChatGPT OAuth consent
  -> app backend credential broker
  -> encrypted per-user refresh token store
  -> per-session fresh access token
  -> subscription-backed LLM config
  -> OpenHands agent-server conversation
  -> ChatGPT Codex backend
```

### Recommended Responsibilities

#### Client or App Backend

- Initiates OAuth.
- Presents consent and terms information.
- Handles browser callback or device-code flow.
- Associates credentials with the authenticated app user.

#### Credential Broker

- Stores refresh tokens encrypted at rest.
- Keeps credentials namespaced per user and tenant.
- Refreshes access tokens server-side.
- Issues only short-lived credential material into execution sessions.
- Supports sign-out and token revocation workflows where possible.

#### Agent-Server Session Creator

- Receives or obtains a short-lived access token.
- Constructs an `LLM` equivalent to the SDK subscription-auth output.
- Avoids storing refresh tokens in workspace containers.
- Ensures credentials are scoped to the correct user and conversation.

#### Agent Runtime

- Uses the configured `LLM` normally.
- Does not need to know the user's refresh token.
- Should not persist OAuth material into project workspaces, logs, traces, or exported artifacts.

---

## Hosted Implementation Sketch

A hosted implementation can either call the existing SDK auth path or replicate the final `create_llm()` construction using credentials from a secure broker.

### Option A: Use SDK CredentialStore Per User

Use a per-user credential directory, mounted or configured only for that user/session.

Conceptually:

```text
/secure-auth-store/
  user_123/
    openai_oauth.json
  user_456/
    openai_oauth.json
```

Then construct a `CredentialStore` for that user's path and call the auth helper.

This is closer to SDK defaults but still requires careful isolation.

### Option B: Credential Broker Constructs the LLM

The credential broker refreshes tokens and then the server constructs the `LLM` directly:

```python
from openhands.sdk import LLM

llm = LLM(
    model="openai/gpt-5.2-codex",
    base_url="https://chatgpt.com/backend-api/codex",
    api_key=access_token,
    extra_headers={
        "originator": "codex_cli_rs",
        "OpenAI-Beta": "responses=experimental",
        "User-Agent": "your-app-name",
        "chatgpt-account-id": chatgpt_account_id,
    },
    litellm_extra_body={"store": False},
    temperature=None,
    max_output_tokens=None,
    stream=True,
)
llm._is_subscription = True
```

This is more invasive because it depends on OpenHands internal subscription behavior. It may be better to wrap or upstream a public constructor for “create subscription LLM from OAuth credentials” rather than relying on private attributes.

---

## Key Design Cautions

### 1. Access Token vs Refresh Token

An access token is short-lived and can be passed into a run with lower risk. A refresh token is long-lived and should not be copied into an agent workspace.

For hosted systems, the refresh token belongs in a credential broker, not in an agent-server workspace.

### 2. `~/.openhands/auth/` Is User-Scoped

The default SDK credential path is user-local. In a multi-user service, a shared filesystem home breaks the security model.

Each user needs separate credential storage.

### 3. Agent Workspaces Are Not Credential Vaults

Agent workspaces are for code and task execution. They should not become long-lived auth stores.

Avoid:

```text
/workspace/.openhands/auth/openai_oauth.json
```

Prefer:

```text
/secure-credential-store/{tenant}/{user}/openai_oauth.json
```

or a real encrypted secret store.

### 4. Token Refresh Needs an Owner

Long-running conversations may outlive an access token.

Decide explicitly where refresh happens:

- SDK local process
- remote agent-server process
- credential broker

For hosted systems, prefer the credential broker.

### 5. Logs and Traces Need Redaction

Subscription auth turns the OAuth access token into `api_key` on an `LLM` object. Any settings dumps, traces, debug logs, or telemetry must redact it.

---

## Recommendation by Deployment Type

| Deployment type | Recommended auth pattern |
|---|---|
| Local SDK only | `LLM.subscription_login(...)` with browser OAuth |
| Local SDK + local Docker `agent-server` | `LLM.subscription_login(...)` and persist `~/.openhands/auth/` |
| Self-hosted remote server | Device-code login with persistent per-user auth volume |
| Hosted single-tenant | Per-user auth directory or credential broker |
| Hosted multi-tenant | Credential broker with encrypted per-user refresh tokens and short-lived access-token injection |

---

## Best Initial Implementation for OpenSymphony-Style Use

For an OpenSymphony-style orchestrator using OpenHands agent-server, the pragmatic roadmap is:

### Phase 1: Personal Local Support

- Allow the orchestrator to select an OpenAI subscription-backed model.
- Call `LLM.subscription_login(vendor="openai", model="gpt-5.2-codex")`.
- Reuse SDK credential storage.
- Support local Docker or remote workspace execution.
- Document that this is single-user local/self-hosted behavior.

### Phase 2: Headless Support

- Add `auth_method="device_code"` as an option.
- Surface the verification URL and one-time code in the orchestrator UI.
- Persist credentials in a user-specific auth directory.
- Add a sign-out action that deletes stored OpenAI credentials.

### Phase 3: Hosted Credential Broker

- Move refresh-token storage out of the agent-server runtime.
- Store encrypted credentials per user and tenant.
- Refresh access tokens before session creation.
- Inject only short-lived access credentials into the `LLM` config.
- Avoid writing OAuth credentials into agent workspaces.
- Add audit logs for sign-in, token refresh, and sign-out.

### Phase 4: Upstream-Friendly SDK Extension

Consider contributing or requesting an SDK API like:

```python
LLM.from_openai_subscription_credentials(
    access_token=...,
    refresh_token=None,
    model="gpt-5.2-codex",
)
```

or:

```python
OpenAISubscriptionAuth.create_llm_from_access_token(...)
```

This would avoid relying on private fields like `_is_subscription` in hosted integrations.

---

## Minimal Agent Handoff Summary

Use `LLM.subscription_login()` when the same process that creates the OpenHands `Agent` can own OAuth credentials.

For personal or self-hosted use, persist `~/.openhands/auth/` and use browser or device-code login.

For hosted multi-user use, do not store refresh tokens inside shared agent-server containers or workspaces. Use a credential broker that stores refresh tokens per user, refreshes access tokens, and constructs a subscription-backed `LLM` for each conversation.

The reason this works with `openhands agent-server` is that subscription authentication ultimately produces a standard OpenHands SDK `LLM` object. The remote `agent-server` does not need a special OAuth UI path as long as the SDK side, server side, or credential broker can construct the correct subscription-backed `LLM` object before the conversation runs.
