//! Explicit ACP extension registrations. A profile opts into reviewed wire contracts;
//! an agent's advertised support never grants an operator permission by itself.
use serde_json::{Value, json};

use super::AcpProfile;
pub use crate::opensymphony_gateway_schema::capability::HarnessOperationCapability as OperationCapability;

pub const CURSOR_VERSION: &str = "cursor@2026.09.08-6caf4ff";
pub const FIXTURE_ECHO_VERSION: &str = "fixture_echo@1";
pub const FIXTURE_ECHO_OPERATION: &str = "fixture.echo";
pub const FIXTURE_ECHO_METHOD: &str = "_opensymphony.test/echo";

fn metadata_schema() -> Value {
    json!({
        "type": "object",
        "maxProperties": 8,
        "propertyNames": {
            "allOf": [
                {"maxLength": 128},
                {"anyOf": [{"pattern": "/"}, {"enum": ["traceparent", "tracestate", "baggage"]}]}
            ]
        }
    })
}

pub fn cursor_enabled(profile: &AcpProfile) -> bool {
    profile.extensions.iter().any(|id| id == CURSOR_VERSION)
}

pub fn cursor_notification(method: &str, params: &Value) -> bool {
    let Some(object) = params.as_object() else {
        return false;
    };
    if object.len() > 16 || !serde_json::to_vec(params).is_ok_and(|bytes| bytes.len() <= 64 * 1024)
    {
        return false;
    }
    let bounded = |key: &str, max: usize| {
        object
            .get(key)
            .and_then(Value::as_str)
            .is_some_and(|value| !value.is_empty() && value.len() <= max)
    };
    match method {
        "cursor/update_todos" => {
            bounded("toolCallId", 128)
                && object
                    .get("todos")
                    .and_then(Value::as_array)
                    .is_some_and(|todos| {
                        todos.len() <= 128
                            && todos.iter().all(|todo| {
                                todo.get("id")
                                    .and_then(Value::as_str)
                                    .is_some_and(|id| !id.is_empty() && id.len() <= 128)
                                    && todo
                                        .get("content")
                                        .and_then(Value::as_str)
                                        .is_some_and(|content| content.len() <= 2048)
                                    && todo.get("status").and_then(Value::as_str).is_some_and(
                                        |status| {
                                            matches!(
                                                status,
                                                "pending"
                                                    | "in_progress"
                                                    | "completed"
                                                    | "cancelled"
                                            )
                                        },
                                    )
                            })
                    })
                && object.get("merge").is_some_and(Value::is_boolean)
        }
        "cursor/task" => {
            bounded("toolCallId", 128)
                && bounded("description", 2048)
                && bounded("prompt", 8192)
                && object.get("subagentType").is_some()
        }
        "cursor/generate_image" => {
            bounded("toolCallId", 128) && bounded("description", 2048) && bounded("filePath", 4096)
        }
        _ => false,
    }
}

pub fn fixture_echo_enabled(profile: &AcpProfile, initialization: &Value) -> bool {
    profile
        .extensions
        .iter()
        .any(|id| id == FIXTURE_ECHO_VERSION)
        && initialization.pointer("/agentCapabilities/_meta/opensymphony.dev~1fixtureEcho")
            == Some(&json!(1))
}

pub fn outbound_operation(
    profile: &AcpProfile,
    initialization: &Value,
    operation_id: &str,
) -> Option<OperationCapability> {
    if operation_id != FIXTURE_ECHO_OPERATION || !fixture_echo_enabled(profile, initialization) {
        return None;
    }
    Some(OperationCapability {
        operation_id: FIXTURE_ECHO_OPERATION.into(),
        namespace: "opensymphony.test".into(),
        version: "1".into(),
        capability_predicate: "agentCapabilities._meta['opensymphony.dev/fixtureEcho'] == 1".into(),
        parameters_schema: json!({"type":"object","required":["value"],"properties":{"value":{"type":"string","maxLength":4096},"_meta":metadata_schema()},"additionalProperties":false}),
        result_schema: json!({"type":"object","required":["value"],"properties":{"value":{"type":"string","maxLength":4096},"_meta":metadata_schema()},"additionalProperties":false}),
        deadline_ms: 5000,
        effect: "read_only_idempotent".into(),
    })
}

pub fn configured_operations(profile: &AcpProfile) -> Vec<OperationCapability> {
    if !profile
        .extensions
        .iter()
        .any(|id| id == FIXTURE_ECHO_VERSION)
    {
        return Vec::new();
    }
    // This describes the predicate for IDE negotiation; active use also checks
    // the peer's negotiated capability and the current session binding.
    outbound_operation(
        profile,
        &json!({"agentCapabilities":{"_meta":{"opensymphony.dev/fixtureEcho":1}}}),
        FIXTURE_ECHO_OPERATION,
    )
    .into_iter()
    .collect()
}

pub fn validate_echo_arguments(arguments: &Value) -> bool {
    let Some(object) = arguments.as_object() else {
        return false;
    };
    if object.len() > 2
        || !object.contains_key("value")
        || object.keys().any(|key| key != "value" && key != "_meta")
    {
        return false;
    }
    if !object
        .get("value")
        .and_then(Value::as_str)
        .is_some_and(|value| value.len() <= 4096)
    {
        return false;
    }
    object.get("_meta").is_none_or(|meta| {
        meta.as_object().is_some_and(|fields| {
            fields.len() <= 8
                && fields.iter().all(|(key, value)| {
                    (key.contains('/')
                        || matches!(key.as_str(), "traceparent" | "tracestate" | "baggage"))
                        && key.len() <= 128
                        && serde_json::to_vec(value).is_ok_and(|bytes| bytes.len() <= 4096)
                })
        })
    })
}

pub fn validate_echo_result(result: &Value) -> bool {
    result.as_object().is_some_and(|object| {
        object.len() <= 2
            && object
                .get("value")
                .and_then(Value::as_str)
                .is_some_and(|value| value.len() <= 4096)
            && object.keys().all(|key| key == "value" || key == "_meta")
            && object.get("_meta").is_none_or(|meta| {
                meta.as_object().is_some_and(|fields| {
                    fields.len() <= 8
                        && fields.iter().all(|(key, value)| {
                            (key.contains('/')
                                || matches!(key.as_str(), "traceparent" | "tracestate" | "baggage"))
                                && key.len() <= 128
                                && serde_json::to_vec(value).is_ok_and(|bytes| bytes.len() <= 4096)
                        })
                })
            })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn echo_requires_registered_profile_and_peer_capability() {
        let mut profile: AcpProfile =
            serde_json::from_value(json!({"command":"peer"})).expect("fixture profile");
        let advertised = json!({"agentCapabilities":{"_meta":{"opensymphony.dev/fixtureEcho":1}}});
        assert!(outbound_operation(&profile, &advertised, FIXTURE_ECHO_OPERATION).is_none());
        profile.extensions.push(FIXTURE_ECHO_VERSION.into());
        assert!(outbound_operation(&profile, &json!({}), FIXTURE_ECHO_OPERATION).is_none());
        let operation = outbound_operation(&profile, &advertised, FIXTURE_ECHO_OPERATION)
            .expect("negotiated operation");
        assert_eq!(
            operation.result_schema["properties"]["_meta"]["type"],
            "object"
        );
        assert_eq!(
            operation.result_schema["properties"]["_meta"]["maxProperties"],
            8
        );
        assert!(validate_echo_arguments(
            &json!({"value":"ok","_meta":{"traceparent":"trace"}})
        ));
        assert!(!validate_echo_arguments(
            &json!({"value":"ok","method":"other"})
        ));
        assert!(validate_echo_result(&json!({"value":"ok"})));
        assert!(!validate_echo_result(&json!({"value":42})));
        assert!(!validate_echo_result(&json!({"value":"ok","execute":true})));
        assert!(!validate_echo_arguments(
            &json!({"value":"ok","_meta":{"command":"unsafe"}})
        ));
    }
}
