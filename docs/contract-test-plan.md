# Contract Test Plan

## Overview

This document defines the contract testing strategy for all inter-service communication in the EntreVista AI system. Contract tests validate that request/response shapes match the defined schemas in `contracts/schemas/`.

## Test Types

### 1. Schema Validation Tests

**Purpose**: Ensure all JSON contract examples conform to their JSON Schema definitions.

**Scope**: All files in `contracts/schemas/`

**Validation**:
```bash
make test-contract
```

**Expected Result**: Zero schema, fixture, or mock dry-run errors. Source
cross-reference warnings are expected when the source `services.md` file is not
present in the checkout.

### 2. Contract Test Dry Run Against Mock HTTP Handlers

**Purpose**: Verify that mock implementations of each service endpoint accept valid requests and return valid responses per contract.

**Scope**: All 10 contracts (C-01 through C-10)

**Mock Setup**:
Each contract gets a mock HTTP handler that:
1. Validates incoming request against the contract schema
2. Returns a canned response matching the contract response schema
3. Logs the request/response for verification

**Test Execution**: `scripts/validate_contracts.py` includes a standard-library
mock handler dry run for every single-endpoint contract and every enumerated
dashboard endpoint. Each dry run builds a valid request, verifies required auth
headers, verifies `X-Correlation-ID` propagation, and verifies missing auth is
rejected for protected contracts.

### 3. Auth Header Validation Tests

**Purpose**: Verify that auth headers are correctly implemented.

**Tests**:
| Contract | Auth Type | Test Case |
|----------|-----------|-----------|
| C-01 to C-06 | X-Internal-Secret | Valid secret → 200/201/202 |
| C-01 to C-06 | X-Internal-Secret | Missing secret → 401 |
| C-01 to C-06 | X-Internal-Secret | Invalid secret → 401 |
| C-07 | None (public) | No auth → 200 (login attempt) |
| C-07 | None (public) | Brute force (5 failures) → 429 |
| C-08 to C-10 | Bearer JWT | Valid JWT → 200 |
| C-08 to C-10 | Bearer JWT | Expired JWT → 401 |
| C-08 to C-10 | Bearer JWT | Missing JWT → 401 |
| C-08 to C-10 | Bearer JWT | Invalid signature → 401 |
| C-08 to C-10 | Bearer JWT | Wrong tenant_id in JWT → 403 |

### 4. Correlation ID Propagation Tests

**Purpose**: Verify that X-Correlation-ID is propagated through service chains.

**Test Scenario**:
```
Telegram → SVC-01 → SVC-02 → SVC-03 (async)
                      → SVC-04 (RAG)
                      → SVC-05 (consent + audit)
```

**Verification**:
1. Generate UUIDv4 as X-Correlation-ID at SVC-01
2. Verify SVC-02 receives the same ID
3. Verify SVC-03, SVC-04, SVC-05 all receive the same ID
4. Verify audit event written by SVC-05 contains the same ID

### 5. Error Envelope Tests

**Purpose**: Verify all services return errors in the standard envelope format.

**Test Cases**:
| Error Code | Scenario | Expected Response Body |
|------------|----------|----------------------|
| VALIDATION_ERROR | Invalid request payload | `{"error": {"code": "VALIDATION_ERROR", "message": "...", "details": {...}}}` |
| UNAUTHORIZED | Missing/invalid auth | `{"error": {"code": "UNAUTHORIZED", "message": "..."}` |
| FORBIDDEN | Valid auth, wrong tenant | `{"error": {"code": "FORBIDDEN", "message": "..."}` |
| NOT_FOUND | Resource doesn't exist | `{"error": {"code": "NOT_FOUND", "message": "..."}` |
| CONFLICT | Duplicate consent | `{"error": {"code": "CONFLICT", "message": "..."}` |
| RATE_LIMITED | Brute force threshold | `{"error": {"code": "RATE_LIMITED", "message": "..."}` |
| INTERNAL_ERROR | Unhandled server error | `{"error": {"code": "INTERNAL_ERROR", "message": "..."}` |
| SERVICE_UNAVAILABLE | Downstream unreachable | `{"error": {"code": "SERVICE_UNAVAILABLE", "message": "..."}` |
| TIMEOUT | Request exceeded budget | `{"error": {"code": "TIMEOUT", "message": "..."}` |

### 6. Tenant Isolation Tests

**Purpose**: Verify that all fixtures and API calls are scoped by `tenant_id`.

**Test Cases**:
| Fixture Type | Test |
|-------------|------|
| Tenant | Cannot access data without explicit `tenant_id` |
| Campaign | Campaign list filtered by `tenant_id` |
| Rubric | Rubric access requires matching `tenant_id` |
| Session | Session data isolated by `tenant_id` |
| Consent | Consent record queries require `tenant_id` |
| Evaluation | Evaluation results filtered by `tenant_id` |
| Audit Event | Audit queries scoped by `tenant_id` |
| Escalation | Escalation alerts filtered by `tenant_id` |
| NPS | NPS submissions scoped by `tenant_id` |
| Document | KB documents isolated by `tenant_id` |

## Fixture-Based Testing

All tests use the fixtures in `contracts/fixtures/`:

| Fixture File | Test Coverage |
|-------------|---------------|
| `tenant_demo.json` | Tenant isolation, multi-tenant scoping |
| `operator_recruiter.json` | Auth flows, JWT validation, role-based access |
| `campaign_software_engineer.json` | Campaign CRUD, RAG search scope |
| `rubric_software_engineer.json` | Evaluation scoring, competency weights |
| `session_happy_path.json` | Full happy path: consent → screening → evaluation |
| `session_escalation.json` | Escalation path: guardrails trigger → human alert |
| `messages_happy_path.json` | Message flow, state transitions |
| `consent_record.json` | Write-once consent, duplicate detection |
| `evaluation_scored.json` | Scoring, citations, human decision flow |
| `audit_events.json` | Immutable audit log, event types |
| `escalation_alert.json` | Alert lifecycle, resolution flow |
| `nps_submission.json` | NPS collection, aggregation |
| `document_knowledge_base.json` | Document upload, embedding, RAG retrieval |

## Test Execution Checklist

- [ ] Schema validation: All 10 contract JSON files validate against JSON Schema draft-07
- [ ] Mock handler tests: All 10 contracts tested against mock HTTP handlers
- [ ] Auth validation: X-Internal-Secret and JWT auth tested for all contracts
- [ ] Correlation ID: UUIDv4 propagation verified through full service chain
- [ ] Error envelopes: All 9 error codes tested with correct response shape
- [ ] Tenant isolation: All 10 fixture types tested with explicit `tenant_id` scoping
- [ ] Cross-reference review: All contracts validated against `services.md` when available

## Running Contract Tests

```bash
# From repository root
make test-contract

# Or directly
python3 scripts/validate_contracts.py

# Include the source-design cross-reference when services.md is available
SERVICES_DOC="Estación 5/agentic_interviewer_ai/aidlc-docs/inception/application-design/services.md" make test-contract
```

## Downstream Service Integration

Each downstream service task MUST:
1. Reference the specific contract artifact(s) from `contracts/schemas/` before coding
2. Implement the mock handler tests for their service endpoints
3. Run `make test-contract` as part of their CI pipeline
4. Validate all request/response shapes against the contract schemas
5. Implement correlation ID propagation per the standard pattern
6. Implement auth validation per the standard pattern (X-Internal-Secret or JWT)
