#!/usr/bin/env python3
"""Validate EntreVista AI shared contracts and fixture coverage.

The checks intentionally use only Python's standard library so downstream
service repos can copy the script before they have chosen a test stack.
"""

from __future__ import annotations

import json
import os
import re
import sys
from dataclasses import dataclass
from pathlib import Path
from typing import Any


CONTRACTS_DIR = Path("contracts/schemas")
FIXTURES_DIR = Path("contracts/fixtures")
DEFAULT_SERVICES_DOC = Path(
    "Estación 5/agentic_interviewer_ai/aidlc-docs/inception/application-design/services.md"
)

EXPECTED_CONTRACT_IDS = {f"C-{index:02d}" for index in range(1, 11)}
INTERNAL_CONTRACT_IDS = {f"C-{index:02d}" for index in range(1, 7)}
DASHBOARD_CONTRACT_IDS = {"C-08", "C-09", "C-10"}
PUBLIC_CONTRACT_IDS = {"C-07"}

EXPECTED_FIXTURES = {
    "audit_events.json",
    "campaign_software_engineer.json",
    "consent_record.json",
    "document_knowledge_base.json",
    "escalation_alert.json",
    "evaluation_scored.json",
    "messages_happy_path.json",
    "nps_submission.json",
    "operator_recruiter.json",
    "rubric_software_engineer.json",
    "session_escalation.json",
    "session_happy_path.json",
    "tenant_demo.json",
}

STANDARD_ERROR_CODES = {
    "VALIDATION_ERROR",
    "UNAUTHORIZED",
    "FORBIDDEN",
    "NOT_FOUND",
    "CONFLICT",
    "RATE_LIMITED",
    "INTERNAL_ERROR",
    "SERVICE_UNAVAILABLE",
    "TIMEOUT",
}

errors: list[str] = []
warnings: list[str] = []


def load_json(path: Path) -> Any:
    try:
        return json.loads(path.read_text())
    except json.JSONDecodeError as exc:
        errors.append(f"{path}: invalid JSON - {exc}")
        return None


def fail(message: str) -> None:
    errors.append(message)
    print(f"  X {message}")


def warn(message: str) -> None:
    warnings.append(message)
    print(f"  ! {message}")


def services_doc_path() -> Path:
    return Path(os.environ.get("SERVICES_DOC", DEFAULT_SERVICES_DOC))


def ok(message: str) -> None:
    print(f"  OK {message}")


def require(condition: bool, message: str) -> None:
    if not condition:
        fail(message)


def as_object(value: Any) -> dict[str, Any]:
    return value if isinstance(value, dict) else {}


def non_null_type(schema: dict[str, Any]) -> str | None:
    schema_type = schema.get("type")
    if isinstance(schema_type, list):
        return next((item for item in schema_type if item != "null"), None)
    return schema_type if isinstance(schema_type, str) else None


def sample_value(schema: dict[str, Any]) -> Any:
    schema_type = non_null_type(schema)
    if "enum" in schema and schema["enum"]:
        return schema["enum"][0]
    if schema_type == "object":
        properties = as_object(schema.get("properties"))
        required = schema.get("required", properties.keys())
        return {
            key: sample_value(as_object(properties.get(key, {"type": "string"})))
            for key in required
        }
    if schema_type == "array":
        return [sample_value(as_object(schema.get("items", {"type": "object"})))]
    if schema_type == "integer":
        return schema.get("default", schema.get("minimum", 1))
    if schema_type == "number":
        return schema.get("default", schema.get("minimum", 0.75))
    if schema_type == "boolean":
        return True
    if schema.get("format") == "date-time":
        return "2025-05-20T14:00:00Z"
    if schema.get("format") == "email":
        return "operator@example.com"
    if schema.get("format") == "uuid":
        return "11111111-1111-4111-8111-111111111111"
    return "sample"


def success_status(response: dict[str, Any]) -> str | None:
    for status in sorted(response):
        if re.fullmatch(r"2\d\d", status):
            return status
    return None


def error_code(payload: dict[str, Any]) -> str | None:
    error = as_object(payload.get("error"))
    code = error.get("code")
    return code if isinstance(code, str) else None


@dataclass(frozen=True)
class MockRequest:
    method: str
    path: str
    headers: dict[str, str]
    body: Any


@dataclass(frozen=True)
class MockResponse:
    status: int
    headers: dict[str, str]
    body: Any


class MockContractHandler:
    def __init__(self, contract: dict[str, Any], endpoint: dict[str, Any] | None = None):
        self.contract = contract
        self.endpoint = endpoint or {}
        self.metadata = as_object(contract.get("metadata"))
        self.contract_id = self.metadata.get("contractId", "")

    def handle(self, request: MockRequest) -> MockResponse:
        expected_method = (self.endpoint.get("method") or self.metadata.get("method", "")).split(
            "/"
        )[0]
        expected_path = self.endpoint.get("path") or self.metadata.get("path", "")
        correlation_id = request.headers.get("X-Correlation-ID")

        if expected_method and request.method != expected_method:
            return self.error(405, "VALIDATION_ERROR", correlation_id)
        if expected_path and request.path != expected_path:
            return self.error(404, "NOT_FOUND", correlation_id)
        if self.contract_id in INTERNAL_CONTRACT_IDS:
            if request.headers.get("X-Internal-Secret") != "test-internal-secret":
                return self.error(401, "UNAUTHORIZED", correlation_id)
        if self.contract_id in DASHBOARD_CONTRACT_IDS:
            if not request.headers.get("Authorization", "").startswith("Bearer "):
                return self.error(401, "UNAUTHORIZED", correlation_id)
        if requires_correlation_id(self.contract) and not correlation_id:
            return self.error(400, "VALIDATION_ERROR", correlation_id)

        status = int(success_status(as_object(self.contract.get("response"))) or "200")
        return MockResponse(
            status=status,
            headers={"X-Correlation-ID": correlation_id or "generated-correlation-id"},
            body={"ok": True, "contract_id": self.contract_id},
        )

    @staticmethod
    def error(status: int, code: str, correlation_id: str | None) -> MockResponse:
        return MockResponse(
            status=status,
            headers={"X-Correlation-ID": correlation_id or "generated-correlation-id"},
            body={"error": {"code": code, "message": code}},
        )


def request_body_for(contract: dict[str, Any], endpoint: dict[str, Any] | None = None) -> Any:
    if endpoint and isinstance(endpoint.get("body"), dict):
        return sample_value(endpoint["body"])
    request = as_object(contract.get("request"))
    body = as_object(request.get("body"))
    return sample_value(body) if body else {}


def headers_for(contract_id: str) -> dict[str, str]:
    headers = {
        "Content-Type": "application/json",
        "X-Correlation-ID": "11111111-1111-4111-8111-111111111111",
    }
    if contract_id in INTERNAL_CONTRACT_IDS:
        headers["X-Internal-Secret"] = "test-internal-secret"
    if contract_id in DASHBOARD_CONTRACT_IDS:
        headers["Authorization"] = "Bearer test-jwt"
    return headers


def requires_correlation_id(contract: dict[str, Any]) -> bool:
    request = as_object(contract.get("request"))
    headers = as_object(request.get("headers"))
    return (
        as_object(headers.get("X-Correlation-ID")).get("required") is True
        or request.get("correlation_id_required") is True
    )


def validate_contract_schema(path: Path, contract: dict[str, Any]) -> str | None:
    required = {"$schema", "title", "metadata", "request", "response"}
    missing = sorted(required - set(contract))
    if missing:
        fail(f"{path.name}: missing top-level fields {missing}")
        return None

    metadata = as_object(contract["metadata"])
    contract_id = metadata.get("contractId")
    if not isinstance(contract_id, str):
        fail(f"{path.name}: metadata.contractId missing")
        return None

    require(contract_id in EXPECTED_CONTRACT_IDS, f"{path.name}: unexpected contract id {contract_id}")
    require(contract_id.lower().replace("-", "") in path.stem, f"{path.name}: filename matches {contract_id}")
    require(bool(metadata.get("from")), f"{path.name}: metadata.from present")
    require(bool(metadata.get("to")), f"{path.name}: metadata.to present")
    require(bool(metadata.get("method")), f"{path.name}: metadata.method present")
    require(bool(metadata.get("path")), f"{path.name}: metadata.path present")
    require(bool(metadata.get("auth")), f"{path.name}: metadata.auth present")
    require(bool(metadata.get("timeout")), f"{path.name}: metadata.timeout present")
    require(success_status(as_object(contract.get("response"))) is not None, f"{path.name}: has 2xx response")

    request = as_object(contract.get("request"))
    if contract_id in INTERNAL_CONTRACT_IDS:
        headers = as_object(request.get("headers"))
        require(
            as_object(headers.get("X-Internal-Secret")).get("required") is True,
            f"{path.name}: requires X-Internal-Secret",
        )
        require(
            as_object(headers.get("X-Correlation-ID")).get("required") is True,
            f"{path.name}: requires X-Correlation-ID",
        )
    elif contract_id in DASHBOARD_CONTRACT_IDS:
        require(
            "Bearer" in str(request.get("auth_header", "")),
            f"{path.name}: documents Bearer JWT auth header",
        )
        require(
            request.get("correlation_id_required") is True,
            f"{path.name}: marks correlation ID as required",
        )
        require(
            bool(contract.get("endpoints")),
            f"{path.name}: enumerates dashboard endpoints",
        )
    elif contract_id in PUBLIC_CONTRACT_IDS:
        headers = as_object(request.get("headers"))
        require(
            as_object(headers.get("X-Correlation-ID")).get("required") is True,
            f"{path.name}: public dashboard login still requires X-Correlation-ID",
        )

    for status, payload in as_object(contract.get("response")).items():
        if re.fullmatch(r"[45]\d\d", status):
            code = error_code(as_object(payload))
            require(code in STANDARD_ERROR_CODES, f"{path.name}: {status} uses standard error code")

    ok(f"{path.name}: schema, auth, timeout, and error envelopes")
    return contract_id


def validate_mock_handler(path: Path, contract: dict[str, Any], contract_id: str) -> None:
    endpoints = contract.get("endpoints")
    endpoint_list = endpoints if isinstance(endpoints, list) else [None]

    for endpoint in endpoint_list:
        endpoint_obj = endpoint if isinstance(endpoint, dict) else None
        method = (
            endpoint_obj.get("method")
            if endpoint_obj
            else as_object(contract.get("metadata")).get("method", "POST")
        )
        method = str(method).split("/")[0]
        path_value = (
            endpoint_obj.get("path")
            if endpoint_obj
            else as_object(contract.get("metadata")).get("path", "")
        )
        request = MockRequest(
            method=method,
            path=str(path_value),
            headers=headers_for(contract_id),
            body=request_body_for(contract, endpoint_obj),
        )
        response = MockContractHandler(contract, endpoint_obj).handle(request)
        require(200 <= response.status < 300, f"{path.name}: mock accepts valid {method} {path_value}")
        require(
            response.headers.get("X-Correlation-ID") == request.headers["X-Correlation-ID"],
            f"{path.name}: mock propagates X-Correlation-ID for {method} {path_value}",
        )

        if contract_id not in PUBLIC_CONTRACT_IDS:
            bad_headers = dict(request.headers)
            bad_headers.pop("X-Internal-Secret", None)
            bad_headers.pop("Authorization", None)
            bad_response = MockContractHandler(contract, endpoint_obj).handle(
                MockRequest(request.method, request.path, bad_headers, request.body)
            )
            require(
                bad_response.status == 401,
                f"{path.name}: mock rejects missing auth for {method} {path_value}",
            )

    ok(f"{path.name}: mock handler dry run")


def fixture_items(value: Any) -> list[dict[str, Any]]:
    if isinstance(value, list):
        return [item for item in value if isinstance(item, dict)]
    if isinstance(value, dict):
        return [value]
    return []


def validate_fixtures() -> None:
    print("\n=== Fixture Validation ===")
    seen = {path.name for path in FIXTURES_DIR.glob("*.json")}
    missing = sorted(EXPECTED_FIXTURES - seen)
    extra = sorted(seen - EXPECTED_FIXTURES)
    require(not missing, f"missing expected fixtures: {missing}")
    require(not extra, f"unexpected fixture files: {extra}")

    for path in sorted(FIXTURES_DIR.glob("*.json")):
        data = load_json(path)
        if data is None:
            continue
        items = fixture_items(data)
        require(bool(items), f"{path.name}: fixture is object or array of objects")
        require(
            all("tenant_id" in item and item["tenant_id"] for item in items),
            f"{path.name}: every fixture item has tenant_id",
        )
        ok(f"{path.name}: tenant-scoped fixture")


def validate_services_doc(contract_ids: set[str]) -> None:
    print("\n=== Source Cross-Reference ===")
    source_path = services_doc_path()
    if not source_path.exists():
        warn(
            f"{source_path}: source services.md not found; set SERVICES_DOC to "
            "enable source cross-reference"
        )
        return

    source = source_path.read_text()
    for contract_id in sorted(contract_ids):
        require(
            f"Contract {contract_id}" in source,
            f"services.md references {contract_id}",
        )
    ok("services.md references all contract IDs")


def main() -> int:
    print("=== Contract Schema Validation ===")
    contract_ids: set[str] = set()
    for path in sorted(CONTRACTS_DIR.glob("*.json")):
        contract = load_json(path)
        if contract is None:
            continue
        contract_id = validate_contract_schema(path, contract)
        if contract_id:
            contract_ids.add(contract_id)
            validate_mock_handler(path, contract, contract_id)

    require(contract_ids == EXPECTED_CONTRACT_IDS, "contract set covers C-01 through C-10")
    validate_fixtures()
    validate_services_doc(contract_ids)

    print("\n=== Summary ===")
    if errors:
        print(f"ERRORS: {len(errors)}")
        for error in errors:
            print(f"  - {error}")
        return 1
    if warnings:
        print(f"WARNINGS: {len(warnings)}")
        for message in warnings:
            print(f"  - {message}")
    print("All contract schemas, fixtures, and mock-handler dry runs passed.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
