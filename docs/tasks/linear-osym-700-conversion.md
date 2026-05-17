# OSYM-700+ Linear Conversion Mapping

Converted at: 2026-05-11T17:15:45-05:00

Project: [OpenSymphony-bootstrap](https://linear.app/trilogy-ai-coe/project/opensymphony-bootstrap-e7b957855cb7)

## Milestones

| Local milestone | Linear milestone ID | Goal |
| --- | --- | --- |
| M6: Gateway And Stream Contract | `c32b5191-7349-4f30-bd46-6340af6da3e5` | Establish the versioned OpenSymphony Gateway, public DTOs, replayable event streams, action receipts, and feasibility baselines for desktop, web, hosted, and high-throughput transports. |
| M7: Shared Client And Desktop Alpha | `dce95c78-504e-42d0-823f-227e68fb395b` | Build the shared TypeScript client foundation and Tauri desktop shell that can connect to local and hosted OpenSymphony profiles through a common frontend contract. |
| M8: Task Graph Operations And OpenHands Run UI | `1b243af1-5678-431b-a66e-acae0b1ac2f5` | Provide Linear-native task graph operations and a rich OpenHands runtime interface with timelines, streams, diffs, validation evidence, approvals, and run actions. |
| M9: Collaborative Planning Alpha | `806afecc-4a9f-4862-8330-6ce70d606058` | Implement the adapted GSD-2 task-creation workflow as a reviewable OpenSymphony planning workspace that produces Linear milestones, issues, sub-issues, dependencies, acceptance criteria, verification expectations, and publish payloads. |
| M10: Web Client And External Gateway | `58d8ef29-ea13-4cd2-b7a5-3768070e8a01` | Deploy the shared frontend as a browser app that connects to local, external, and hosted gateways with reconnect-safe remote transport behavior. |
| M11: Hosted Alpha | `4a82a4eb-1782-47a1-8a47-7afb0210cb76` | Add hosted multi-user execution where server-owned runs continue after clients disconnect and permissions, secrets, workspaces, audit, and administration are enforced centrally. |
| M12: Provider, Harness, And Model Readiness | `5f7c2983-86fb-4ea3-8c91-de017a8698a3` | Add the model, credential, and harness seams for OpenAI ChatGPT/Codex subscription-backed OpenHands use, feature-gated Codex app-server prototypes, and future cross-harness routing. |
| M13: Hardening And Release Quality | `0b105fe4-91be-42c2-98b3-75895d4de095` | Prove the system through contract, end-to-end, performance, security, accessibility, and documentation work. |

## Issues

| Local task | Linear issue | Milestone | Priority | Estimate | Blocked by | Blocks |
| --- | --- | --- | --- | --- | --- | --- |
| `OSYM-700` | [COE-389: Current Gateway Inventory And Vocabulary](https://linear.app/trilogy-ai-coe/issue/COE-389/current-gateway-inventory-and-vocabulary) | M6: Gateway And Stream Contract | Urgent | 3 | None | COE-390, COE-391, COE-392 |
| `OSYM-701` | [COE-390: Gateway Schemas And Stream Feasibility](https://linear.app/trilogy-ai-coe/issue/COE-390/gateway-schemas-and-stream-feasibility) | M6: Gateway And Stream Contract | Urgent | 5 | COE-389 | COE-391, COE-392, COE-393, COE-394, COE-403, COE-410, COE-407 |
| `OSYM-702` | [COE-391: Gateway Module, Capabilities, And Dashboard Snapshot](https://linear.app/trilogy-ai-coe/issue/COE-391/gateway-module-capabilities-and-dashboard-snapshot) | M6: Gateway And Stream Contract | Urgent | 5 | COE-389, COE-390 | COE-392, COE-393, COE-396, COE-394 |
| `OSYM-703` | [COE-392: Task Graph, Run Detail, File, And Diff Read APIs](https://linear.app/trilogy-ai-coe/issue/COE-392/task-graph-run-detail-file-and-diff-read-apis) | M6: Gateway And Stream Contract | Urgent | 8 | COE-390, COE-391 | COE-399, COE-400, COE-412, COE-414 |
| `OSYM-704` | [COE-393: Event Journal And Stream Broker](https://linear.app/trilogy-ai-coe/issue/COE-393/event-journal-and-stream-broker) | M6: Gateway And Stream Contract | Urgent | 8 | COE-390, COE-391 | COE-396, COE-397, COE-412, COE-407, COE-424 |
| `OSYM-705` | [COE-396: Action Receipts And Initial Run Actions](https://linear.app/trilogy-ai-coe/issue/COE-396/action-receipts-and-initial-run-actions) | M6: Gateway And Stream Contract | Urgent | 5 | COE-391, COE-393 | COE-405, COE-414, COE-418, COE-420 |
| `OSYM-710` | [COE-394: Frontend Workspace And Shared Schemas](https://linear.app/trilogy-ai-coe/issue/COE-394/frontend-workspace-and-shared-schemas) | M7: Shared Client And Desktop Alpha | Urgent | 5 | COE-390, COE-391 | COE-397, COE-402, COE-403, COE-398, COE-401 |
| `OSYM-711` | [COE-397: Gateway API Client, Transport Adapters, And Reducers](https://linear.app/trilogy-ai-coe/issue/COE-397/gateway-api-client-transport-adapters-and-reducers) | M7: Shared Client And Desktop Alpha | Urgent | 8 | COE-393, COE-394 | COE-402, COE-403, COE-410, COE-407 |
| `OSYM-712` | [COE-402: App Shell, Dashboard, Task Graph, And Run Views](https://linear.app/trilogy-ai-coe/issue/COE-402/app-shell-dashboard-task-graph-and-run-views) | M7: Shared Client And Desktop Alpha | High | 8 | COE-392, COE-394, COE-397 | COE-411, COE-414, COE-417, COE-419 |
| `OSYM-713` | [COE-403: Terminal And Log Renderer Prototype](https://linear.app/trilogy-ai-coe/issue/COE-403/terminal-and-log-renderer-prototype) | M7: Shared Client And Desktop Alpha | High | 5 | COE-390, COE-397 | COE-410, COE-412 |
| `OSYM-714` | [COE-398: Tauri Shell And Security Capabilities](https://linear.app/trilogy-ai-coe/issue/COE-398/tauri-shell-and-security-capabilities) | M7: Shared Client And Desktop Alpha | High | 5 | COE-394 | COE-404, COE-409, COE-410 |
| `OSYM-715` | [COE-404: Desktop Connection Profiles And Daemon Management](https://linear.app/trilogy-ai-coe/issue/COE-404/desktop-connection-profiles-and-daemon-management) | M7: Shared Client And Desktop Alpha | High | 5 | COE-391, COE-397, COE-398 | COE-409, COE-410 |
| `OSYM-716` | [COE-409: Desktop Settings, Keychain, And Native Actions](https://linear.app/trilogy-ai-coe/issue/COE-409/desktop-settings-keychain-and-native-actions) | M7: Shared Client And Desktop Alpha | Normal | 5 | COE-398, COE-404 | COE-423 |
| `OSYM-717` | [COE-410: Desktop Local Stream Optimization](https://linear.app/trilogy-ai-coe/issue/COE-410/desktop-local-stream-optimization) | M7: Shared Client And Desktop Alpha | Normal | 8 | COE-393, COE-397, COE-403, COE-404 | COE-431 |
| `OSYM-720` | [COE-399: Linear Read Coverage And Task Graph Cache](https://linear.app/trilogy-ai-coe/issue/COE-399/linear-read-coverage-and-task-graph-cache) | M8: Task Graph Operations And OpenHands Run UI | Urgent | 5 | COE-392 | COE-405, COE-411, COE-406 |
| `OSYM-721` | [COE-405: Linear Milestone, Issue, And Sub-Issue Mutations](https://linear.app/trilogy-ai-coe/issue/COE-405/linear-milestone-issue-and-sub-issue-mutations) | M8: Task Graph Operations And OpenHands Run UI | Urgent | 8 | COE-396, COE-399 | COE-411, COE-418 |
| `OSYM-722` | [COE-411: Task Graph Editor And Runtime Overlay UI](https://linear.app/trilogy-ai-coe/issue/COE-411/task-graph-editor-and-runtime-overlay-ui) | M8: Task Graph Operations And OpenHands Run UI | High | 8 | COE-402, COE-399, COE-405 | COE-417 |
| `OSYM-723` | [COE-400: OpenHands Event Normalization And Runtime Mirror](https://linear.app/trilogy-ai-coe/issue/COE-400/openhands-event-normalization-and-runtime-mirror) | M8: Task Graph Operations And OpenHands Run UI | Urgent | 8 | COE-392 | COE-412, COE-414, COE-422 |
| `OSYM-724` | [COE-412: Runtime Timeline And Terminal/Log Association](https://linear.app/trilogy-ai-coe/issue/COE-412/runtime-timeline-and-terminallog-association) | M8: Task Graph Operations And OpenHands Run UI | High | 8 | COE-393, COE-403, COE-400 | COE-414, COE-430, COE-431 |
| `OSYM-725` | [COE-414: Diff, Validation, Approval, And Run Action Views](https://linear.app/trilogy-ai-coe/issue/COE-414/diff-validation-approval-and-run-action-views) | M8: Task Graph Operations And OpenHands Run UI | High | 8 | COE-392, COE-396, COE-402, COE-412 | COE-422, COE-430 |
| `OSYM-730` | [COE-395: Planning Artifact Schema And Session Service](https://linear.app/trilogy-ai-coe/issue/COE-395/planning-artifact-schema-and-session-service) | M9: Collaborative Planning Alpha | Urgent | 8 | COE-390, COE-391 | COE-406, COE-413, COE-415, COE-417 |
| `OSYM-731` | [COE-406: Repository, Linear, And Research Analysis](https://linear.app/trilogy-ai-coe/issue/COE-406/repository-linear-and-research-analysis) | M9: Collaborative Planning Alpha | Urgent | 8 | COE-399, COE-395 | COE-413 |
| `OSYM-732` | [COE-413: Implementation Plan Generator Stage](https://linear.app/trilogy-ai-coe/issue/COE-413/implementation-plan-generator-stage) | M9: Collaborative Planning Alpha | Urgent | 5 | COE-406 | COE-415 |
| `OSYM-733` | [COE-415: Milestone, Issue, And Sub-Issue Compiler](https://linear.app/trilogy-ai-coe/issue/COE-415/milestone-issue-and-sub-issue-compiler) | M9: Collaborative Planning Alpha | Urgent | 5 | COE-413 | COE-416, COE-418 |
| `OSYM-734` | [COE-416: Dependency Graph And Plan Checks](https://linear.app/trilogy-ai-coe/issue/COE-416/dependency-graph-and-plan-checks) | M9: Collaborative Planning Alpha | Urgent | 5 | COE-415 | COE-417, COE-418 |
| `OSYM-735` | [COE-417: Planning Workspace UI](https://linear.app/trilogy-ai-coe/issue/COE-417/planning-workspace-ui) | M9: Collaborative Planning Alpha | High | 8 | COE-402, COE-411, COE-395, COE-416 | COE-418, COE-419 |
| `OSYM-736` | [COE-418: Linear Draft Preview And Publish Flow](https://linear.app/trilogy-ai-coe/issue/COE-418/linear-draft-preview-and-publish-flow) | M9: Collaborative Planning Alpha | Urgent | 8 | COE-405, COE-415, COE-416, COE-417 | COE-430 |
| `OSYM-740` | [COE-401: Web App Entry And Deployment Modes](https://linear.app/trilogy-ai-coe/issue/COE-401/web-app-entry-and-deployment-modes) | M10: Web Client And External Gateway | High | 5 | COE-394 | COE-407, COE-419 |
| `OSYM-741` | [COE-407: Browser Transport And Remote Stream Protocols](https://linear.app/trilogy-ai-coe/issue/COE-407/browser-transport-and-remote-stream-protocols) | M10: Web Client And External Gateway | Urgent | 8 | COE-393, COE-397, COE-401 | COE-419, COE-420 |
| `OSYM-742` | [COE-419: Hosted Auth Placeholders And Web Parity](https://linear.app/trilogy-ai-coe/issue/COE-419/hosted-auth-placeholders-and-web-parity) | M10: Web Client And External Gateway | High | 5 | COE-402, COE-417, COE-401, COE-407 | COE-420, COE-431 |
| `OSYM-750` | [COE-420: Hosted Identity, Auth, And RBAC](https://linear.app/trilogy-ai-coe/issue/COE-420/hosted-identity-auth-and-rbac) | M11: Hosted Alpha | Urgent | 8 | COE-396, COE-407, COE-419 | COE-421, COE-422, COE-424, COE-427 |
| `OSYM-751` | [COE-421: Hosted Secrets And Linear Connections](https://linear.app/trilogy-ai-coe/issue/COE-421/hosted-secrets-and-linear-connections) | M11: Hosted Alpha | Urgent | 8 | COE-420 | COE-422, COE-423 |
| `OSYM-752` | [COE-422: Hosted Workspace Isolation And Runtime Pool](https://linear.app/trilogy-ai-coe/issue/COE-422/hosted-workspace-isolation-and-runtime-pool) | M11: Hosted Alpha | Urgent | 13 | COE-400, COE-414, COE-420, COE-421 | COE-424 |
| `OSYM-753` | [COE-424: Client-Independent Run Persistence](https://linear.app/trilogy-ai-coe/issue/COE-424/client-independent-run-persistence) | M11: Hosted Alpha | Urgent | 8 | COE-393, COE-420, COE-422 | COE-427, COE-431 |
| `OSYM-754` | [COE-427: Hosted Audit, Metrics, And Admin Controls](https://linear.app/trilogy-ai-coe/issue/COE-427/hosted-audit-metrics-and-admin-controls) | M11: Hosted Alpha | High | 8 | COE-420, COE-424 | COE-431, COE-432 |
| `OSYM-760` | [COE-408: Harness Adapter And Capability Model](https://linear.app/trilogy-ai-coe/issue/COE-408/harness-adapter-and-capability-model) | M12: Provider, Harness, And Model Readiness | High | 5 | COE-390, COE-400 | COE-423, COE-426, COE-429 |
| `OSYM-761` | [COE-423: Model And Credential Settings](https://linear.app/trilogy-ai-coe/issue/COE-423/model-and-credential-settings) | M12: Provider, Harness, And Model Readiness | High | 8 | COE-409, COE-421, COE-408 | COE-425, COE-428, COE-426 |
| `OSYM-762` | [COE-425: OpenHands Subscription Credential Adapter](https://linear.app/trilogy-ai-coe/issue/COE-425/openhands-subscription-credential-adapter) | M12: Provider, Harness, And Model Readiness | High | 8 | COE-423 | COE-428 |
| `OSYM-763` | [COE-428: Model Configuration UI And Routing Metadata](https://linear.app/trilogy-ai-coe/issue/COE-428/model-configuration-ui-and-routing-metadata) | M12: Provider, Harness, And Model Readiness | Normal | 5 | COE-425 | COE-429 |
| `OSYM-764` | [COE-426: Codex App-Server Prototype And Benchmarks](https://linear.app/trilogy-ai-coe/issue/COE-426/codex-app-server-prototype-and-benchmarks) | M12: Provider, Harness, And Model Readiness | Normal | 8 | COE-408, COE-423 | COE-429 |
| `OSYM-765` | [COE-429: Codex Approvals And Cross-Harness Routing](https://linear.app/trilogy-ai-coe/issue/COE-429/codex-approvals-and-cross-harness-routing) | M12: Provider, Harness, And Model Readiness | Normal | 8 | COE-428, COE-426 | COE-430, COE-431 |
| `OSYM-770` | [COE-430: Contract And Local End-To-End Tests](https://linear.app/trilogy-ai-coe/issue/COE-430/contract-and-local-end-to-end-tests) | M13: Hardening And Release Quality | Urgent | 8 | COE-412, COE-414, COE-418, COE-429 | COE-431, COE-432 |
| `OSYM-771` | [COE-431: Web, Hosted, And Performance Tests](https://linear.app/trilogy-ai-coe/issue/COE-431/web-hosted-and-performance-tests) | M13: Hardening And Release Quality | Urgent | 8 | COE-410, COE-407, COE-424, COE-429, COE-430 | COE-432 |
| `OSYM-772` | [COE-432: Security, Accessibility, Documentation, And Developer Experience](https://linear.app/trilogy-ai-coe/issue/COE-432/security-accessibility-documentation-and-developer-experience) | M13: Hardening And Release Quality | Urgent | 8 | COE-427, COE-431 | None |

## Creation Waves

- Wave 1: COE-389
- Wave 2: COE-390
- Wave 3: COE-391
- Wave 4: COE-392, COE-393, COE-394, COE-395
- Wave 5: COE-396, COE-397, COE-398, COE-399, COE-400, COE-401
- Wave 6: COE-402, COE-403, COE-404, COE-405, COE-406, COE-407, COE-408
- Wave 7: COE-409, COE-410, COE-411, COE-412, COE-413
- Wave 8: COE-414, COE-415
- Wave 9: COE-416
- Wave 10: COE-417
- Wave 11: COE-418, COE-419
- Wave 12: COE-420
- Wave 13: COE-421
- Wave 14: COE-422, COE-423
- Wave 15: COE-424, COE-425, COE-426
- Wave 16: COE-427, COE-428
- Wave 17: COE-429
- Wave 18: COE-430
- Wave 19: COE-431
- Wave 20: COE-432

## Validation

- milestones: 8 present
- issues: 44 present and assigned to milestones
- blockerRelations: 100 applied or already present
- staleLocalIds: none found in created issue descriptions
- parentHierarchy: source hierarchy preserved as milestone-assigned project issues
- projectOverview: updated

## Blocker Relations

- COE-389 blocks COE-390
- COE-389 blocks COE-391
- COE-390 blocks COE-391
- COE-390 blocks COE-392
- COE-391 blocks COE-392
- COE-390 blocks COE-393
- COE-391 blocks COE-393
- COE-391 blocks COE-396
- COE-393 blocks COE-396
- COE-390 blocks COE-394
- COE-391 blocks COE-394
- COE-393 blocks COE-397
- COE-394 blocks COE-397
- COE-392 blocks COE-402
- COE-394 blocks COE-402
- COE-397 blocks COE-402
- COE-390 blocks COE-403
- COE-397 blocks COE-403
- COE-394 blocks COE-398
- COE-391 blocks COE-404
- COE-397 blocks COE-404
- COE-398 blocks COE-404
- COE-398 blocks COE-409
- COE-404 blocks COE-409
- COE-393 blocks COE-410
- COE-397 blocks COE-410
- COE-403 blocks COE-410
- COE-404 blocks COE-410
- COE-392 blocks COE-399
- COE-396 blocks COE-405
- COE-399 blocks COE-405
- COE-402 blocks COE-411
- COE-399 blocks COE-411
- COE-405 blocks COE-411
- COE-392 blocks COE-400
- COE-393 blocks COE-412
- COE-403 blocks COE-412
- COE-400 blocks COE-412
- COE-392 blocks COE-414
- COE-396 blocks COE-414
- COE-402 blocks COE-414
- COE-412 blocks COE-414
- COE-390 blocks COE-395
- COE-391 blocks COE-395
- COE-399 blocks COE-406
- COE-395 blocks COE-406
- COE-406 blocks COE-413
- COE-413 blocks COE-415
- COE-415 blocks COE-416
- COE-402 blocks COE-417
- COE-411 blocks COE-417
- COE-395 blocks COE-417
- COE-416 blocks COE-417
- COE-405 blocks COE-418
- COE-415 blocks COE-418
- COE-416 blocks COE-418
- COE-417 blocks COE-418
- COE-394 blocks COE-401
- COE-393 blocks COE-407
- COE-397 blocks COE-407
- COE-401 blocks COE-407
- COE-402 blocks COE-419
- COE-417 blocks COE-419
- COE-401 blocks COE-419
- COE-407 blocks COE-419
- COE-396 blocks COE-420
- COE-407 blocks COE-420
- COE-419 blocks COE-420
- COE-420 blocks COE-421
- COE-400 blocks COE-422
- COE-414 blocks COE-422
- COE-420 blocks COE-422
- COE-421 blocks COE-422
- COE-393 blocks COE-424
- COE-420 blocks COE-424
- COE-422 blocks COE-424
- COE-420 blocks COE-427
- COE-424 blocks COE-427
- COE-390 blocks COE-408
- COE-400 blocks COE-408
- COE-409 blocks COE-423
- COE-421 blocks COE-423
- COE-408 blocks COE-423
- COE-423 blocks COE-425
- COE-425 blocks COE-428
- COE-408 blocks COE-426
- COE-423 blocks COE-426
- COE-428 blocks COE-429
- COE-426 blocks COE-429
- COE-412 blocks COE-430
- COE-414 blocks COE-430
- COE-418 blocks COE-430
- COE-429 blocks COE-430
- COE-410 blocks COE-431
- COE-407 blocks COE-431
- COE-424 blocks COE-431
- COE-429 blocks COE-431
- COE-430 blocks COE-431
- COE-427 blocks COE-432
- COE-431 blocks COE-432
