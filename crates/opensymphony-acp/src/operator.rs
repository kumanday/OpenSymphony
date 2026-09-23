//! Bounded, typed operator callbacks. No arbitrary ACP method passthrough.
use super::*;
use crate::opensymphony_gateway_schema::approval::{
    OperatorAnswer, OperatorInteraction, OperatorInteractionKind, OperatorOption, OperatorQuestion,
};
use crate::opensymphony_workflow::AcpPermissionPolicy;
use agent_client_protocol::schema::v1::{
    CreateElicitationRequest, ElicitationMode, ElicitationPropertySchema, ElicitationScope,
    MultiSelectItems, RequestPermissionRequest,
};
use chrono::{Duration as ChronoDuration, Utc};
use serde::Deserialize;
use std::{
    collections::{HashMap, HashSet},
    sync::atomic::{AtomicU8, Ordering},
};

/// A failed operator receipt must fence a response still waiting in the worker queue.
/// Claiming delivery and cancelling it are mutually exclusive atomic transitions.
#[derive(Clone, Default)]
pub struct AcpOperatorDeliveryFence(Arc<AtomicU8>);

impl AcpOperatorDeliveryFence {
    pub fn cancel(&self) -> bool {
        self.0
            .compare_exchange(0, 1, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire) == 1
    }

    pub fn claim(&self) -> bool {
        self.0
            .compare_exchange(0, 2, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    }
}

pub struct AcpOperatorRequest {
    pub interaction: OperatorInteraction,
    pub reply: oneshot::Sender<AcpOperatorReply>,
}

pub struct AcpOperatorReply {
    pub answer: OperatorAnswer,
    pub acknowledgement: oneshot::Sender<bool>,
    pub delivery: AcpOperatorDeliveryFence,
}

pub enum AcpOperatorEvent {
    Opened(Box<AcpOperatorRequest>),
    Closed(String),
}

pub(super) type OperatorRouter =
    Arc<Mutex<Option<(Option<mpsc::Sender<AcpOperatorEvent>>, CancellationToken)>>>;

fn bounded(value: &str, max: usize) -> bool {
    !value.trim().is_empty() && value.len() <= max && !value.chars().any(char::is_control)
}

fn no_secret_prompt(value: &str) -> bool {
    // ponytail: keyword filter rejects some benign prompts; use a reviewed
    // classification policy if agents need sensitive multiple-choice input.
    let lower = value.to_ascii_lowercase();
    ![
        "password",
        "secret",
        "token",
        "credential",
        "api key",
        "private key",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CursorQuestionRequest {
    session_id: String,
    tool_call_id: String,
    title: Option<String>,
    questions: Vec<CursorQuestion>,
    #[serde(rename = "_meta", default)]
    meta: Option<Value>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CursorQuestion {
    id: String,
    prompt: String,
    options: Vec<CursorOption>,
    #[serde(default)]
    allow_multiple: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CursorOption {
    id: String,
    label: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CursorPlanRequest {
    session_id: String,
    tool_call_id: String,
    name: Option<String>,
    overview: Option<String>,
    plan: String,
    todos: Vec<CursorTodo>,
    #[serde(default)]
    is_project: bool,
    #[serde(default)]
    phases: Vec<CursorPhase>,
    #[serde(rename = "_meta", default)]
    meta: Option<Value>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CursorPhase {
    name: String,
    todos: Vec<CursorTodo>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CursorTodo {
    id: String,
    content: String,
    status: String,
}

fn valid_cursor_meta(meta: &Option<Value>) -> bool {
    meta.as_ref().is_none_or(|meta| {
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

fn valid_cursor_todo(todo: &CursorTodo) -> bool {
    bounded(&todo.id, 128)
        && bounded(&todo.content, 2048)
        && matches!(
            todo.status.as_str(),
            "pending" | "in_progress" | "completed" | "cancelled"
        )
}

pub(super) fn same_binding_ids(safe: &OperatorInteraction, original: &OperatorInteraction) -> bool {
    let ids = |interaction: &OperatorInteraction| {
        let options = interaction
            .options
            .iter()
            .map(|option| option.id.clone())
            .collect::<HashSet<_>>();
        let questions = interaction
            .questions
            .iter()
            .map(|question| {
                (
                    question.id.clone(),
                    question
                        .options
                        .iter()
                        .map(|option| option.id.clone())
                        .collect::<HashSet<_>>(),
                )
            })
            .collect::<HashMap<_, _>>();
        (options, questions)
    };
    ids(safe) == ids(original)
}

pub(super) fn parse_interaction(
    method: &str,
    params: &Value,
    rpc_id: Value,
    session_id: &str,
    timeout: Duration,
) -> Result<OperatorInteraction, agent_client_protocol::Error> {
    let invalid = agent_client_protocol::Error::invalid_params;
    let (kind, title, options, questions, plan) = match method {
        "session/request_permission" => {
            let request: RequestPermissionRequest =
                serde_json::from_value(params.clone()).map_err(|_| invalid())?;
            if request.session_id.0.as_ref() != session_id
                || request.options.is_empty()
                || request.options.len() > 32
            {
                return Err(invalid());
            }
            let title = request
                .tool_call
                .fields
                .title
                .unwrap_or_else(|| "ACP tool permission".into());
            let options = request
                .options
                .into_iter()
                .map(|option| OperatorOption {
                    id: option.option_id.0.to_string(),
                    label: option.name,
                    kind: serde_json::to_value(option.kind)
                        .ok()
                        .and_then(|v| v.as_str().map(str::to_owned))
                        .unwrap_or_default(),
                })
                .collect::<Vec<_>>();
            (
                OperatorInteractionKind::Permission,
                title,
                options,
                Vec::new(),
                None,
            )
        }
        "elicitation/create" => {
            let request: CreateElicitationRequest =
                serde_json::from_value(params.clone()).map_err(|_| invalid())?;
            let ElicitationMode::Form(form) = request.mode else {
                return Err(invalid()); // URL elicitation needs an out-of-band consent flow.
            };
            let ElicitationScope::Session(scope) = form.scope else {
                return Err(invalid());
            };
            if scope.session_id.0.as_ref() != session_id
                || form.requested_schema.properties.is_empty()
                || form.requested_schema.properties.len() > 8
            {
                return Err(invalid());
            }
            let required = form.requested_schema.required.ok_or_else(invalid)?;
            if required.len() != form.requested_schema.properties.len()
                || required.iter().collect::<HashSet<_>>().len() != required.len()
                || required
                    .iter()
                    .any(|id| !form.requested_schema.properties.contains_key(id))
            {
                return Err(invalid());
            }
            let mut questions = Vec::new();
            for (id, property) in form.requested_schema.properties {
                let (prompt, choices, allow_multiple) = match property {
                    ElicitationPropertySchema::String(field) => {
                        if field.min_length.is_some()
                            || field.max_length.is_some()
                            || field.pattern.is_some()
                            || field.format.is_some()
                        {
                            return Err(invalid());
                        }
                        let choices = match (field.enum_values, field.one_of) {
                            (Some(values), None) => values
                                .into_iter()
                                .map(|value| (value.clone(), value))
                                .collect(),
                            (None, Some(values)) => values
                                .into_iter()
                                .map(|option| (option.value, option.title))
                                .collect(),
                            _ => return Err(invalid()),
                        };
                        (field.title.unwrap_or_else(|| id.clone()), choices, false)
                    }
                    ElicitationPropertySchema::Array(field) => {
                        if field.min_items.is_some() || field.max_items.is_some() {
                            return Err(invalid());
                        }
                        let choices = match field.items {
                            MultiSelectItems::String(items) => items
                                .values
                                .into_iter()
                                .map(|value| (value.clone(), value))
                                .collect(),
                            MultiSelectItems::Titled(items) => items
                                .options
                                .into_iter()
                                .map(|option| (option.value, option.title))
                                .collect(),
                            _ => return Err(invalid()),
                        };
                        (field.title.unwrap_or_else(|| id.clone()), choices, true)
                    }
                    _ => return Err(invalid()),
                };
                let choices: Vec<(String, String)> = choices;
                if !bounded(&id, 128)
                    || !bounded(&prompt, 2048)
                    || !no_secret_prompt(&id)
                    || !no_secret_prompt(&prompt)
                    || choices.is_empty()
                    || choices.len() > 32
                    || choices
                        .iter()
                        .map(|(id, _)| id)
                        .collect::<HashSet<_>>()
                        .len()
                        != choices.len()
                    || choices.iter().any(|(id, label)| {
                        !bounded(id, 128)
                            || !bounded(label, 1024)
                            || !no_secret_prompt(id)
                            || !no_secret_prompt(label)
                    })
                {
                    return Err(invalid());
                }
                questions.push(OperatorQuestion {
                    id,
                    prompt,
                    options: choices
                        .into_iter()
                        .map(|(id, label)| OperatorOption {
                            id,
                            label,
                            kind: "choice".into(),
                        })
                        .collect(),
                    allow_multiple,
                });
            }
            (
                OperatorInteractionKind::Question,
                request.message,
                Vec::new(),
                questions,
                None,
            )
        }
        "cursor/ask_question" => {
            let request: CursorQuestionRequest =
                serde_json::from_value(params.clone()).map_err(|_| invalid())?;
            if request.session_id != session_id
                || !bounded(&request.tool_call_id, 128)
                || !valid_cursor_meta(&request.meta)
                || request.questions.is_empty()
                || request.questions.len() > 8
            {
                return Err(invalid());
            }
            let title = request.title.unwrap_or_else(|| "Cursor question".into());
            let mut ids = HashSet::new();
            let mut questions = Vec::new();
            for question in request.questions {
                if !bounded(&question.id, 128)
                    || !ids.insert(question.id.clone())
                    || !bounded(&question.prompt, 2048)
                    || !no_secret_prompt(&question.prompt)
                    || question.options.is_empty()
                    || question.options.len() > 32
                {
                    return Err(invalid());
                }
                let mut option_ids = HashSet::new();
                let options = question
                    .options
                    .into_iter()
                    .map(|option| {
                        if !bounded(&option.id, 128)
                            || !bounded(&option.label, 1024)
                            || !no_secret_prompt(&option.label)
                            || !option_ids.insert(option.id.clone())
                        {
                            return Err(invalid());
                        }
                        Ok(OperatorOption {
                            id: option.id,
                            label: option.label,
                            kind: "choice".into(),
                        })
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                questions.push(OperatorQuestion {
                    id: question.id,
                    prompt: question.prompt,
                    options,
                    allow_multiple: question.allow_multiple,
                });
            }
            (
                OperatorInteractionKind::Question,
                title,
                Vec::new(),
                questions,
                None,
            )
        }
        "cursor/create_plan" => {
            let request: CursorPlanRequest =
                serde_json::from_value(params.clone()).map_err(|_| invalid())?;
            if request.session_id != session_id
                || !bounded(&request.tool_call_id, 128)
                || !valid_cursor_meta(&request.meta)
                || request.plan.trim().is_empty()
                || request.plan.len() > 64 * 1024
                || request.todos.len() > 128
                || request.phases.len() > 32
                || request.todos.iter().any(|todo| !valid_cursor_todo(todo))
                || request.phases.iter().any(|phase| {
                    !bounded(&phase.name, 1024)
                        || phase.todos.len() > 128
                        || phase.todos.iter().any(|todo| !valid_cursor_todo(todo))
                })
            {
                return Err(invalid());
            }
            let _ = request.is_project;
            let _ = request.overview;
            (
                OperatorInteractionKind::PlanApproval,
                request.name.unwrap_or_else(|| "Cursor plan".into()),
                Vec::new(),
                Vec::new(),
                Some(request.plan),
            )
        }
        _ => return Err(agent_client_protocol::Error::method_not_found()),
    };
    if !bounded(&title, 2048)
        || (kind != OperatorInteractionKind::PlanApproval && !no_secret_prompt(&title))
        || !rpc_id.is_number() && !rpc_id.is_string()
    {
        return Err(invalid());
    }
    let mut option_ids = HashSet::new();
    if options.iter().any(|option| {
        !bounded(&option.id, 128)
            || !bounded(&option.label, 1024)
            || !no_secret_prompt(&option.label)
            || !option_ids.insert(option.id.as_str())
    }) {
        return Err(invalid());
    }
    let requested_at = Utc::now();
    let expires_at = requested_at + ChronoDuration::from_std(timeout).map_err(|_| invalid())?;
    Ok(OperatorInteraction {
        request_id: uuid::Uuid::new_v4().to_string(),
        run_id: String::new(),
        issue_id: String::new(),
        issue_identifier: String::new(),
        session_id: session_id.into(),
        generation: 0,
        // The SDK responder retains the exact peer ID privately. Public
        // clients echo only this generated binding token, never peer input.
        rpc_id: uuid::Uuid::new_v4().to_string(),
        kind,
        title,
        options,
        questions,
        plan,
        requested_at,
        expires_at,
    })
}

pub(super) fn automatic_answer(
    policy: AcpPermissionPolicy,
    interaction: &OperatorInteraction,
) -> Option<OperatorAnswer> {
    if interaction.kind != OperatorInteractionKind::Permission {
        return None;
    }
    let kind = match policy {
        AcpPermissionPolicy::Operator => return None,
        AcpPermissionPolicy::Deny => "reject_once",
        AcpPermissionPolicy::AllowOnce => "allow_once",
    };
    Some(
        interaction
            .options
            .iter()
            .find(|option| option.kind == kind)
            .map(|option| OperatorAnswer::Permission {
                option_id: option.id.clone(),
            })
            .unwrap_or(OperatorAnswer::Cancel),
    )
}

pub(super) fn response_for(interaction: &OperatorInteraction, answer: OperatorAnswer) -> Value {
    match (interaction.kind, answer) {
        (OperatorInteractionKind::Permission, OperatorAnswer::Permission { option_id })
            if interaction
                .options
                .iter()
                .any(|option| option.id == option_id) =>
        {
            json!({"outcome": {"outcome": "selected", "optionId": option_id}})
        }
        (OperatorInteractionKind::Permission, _) => json!({"outcome": {"outcome": "cancelled"}}),
        (OperatorInteractionKind::Question, OperatorAnswer::Question { answers }) => {
            let valid = answers.len() == interaction.questions.len()
                && interaction.questions.iter().all(|question| {
                    answers
                        .iter()
                        .filter(|answer| answer.question_id == question.id)
                        .count()
                        == 1
                        && answers
                            .iter()
                            .find(|answer| answer.question_id == question.id)
                            .is_some_and(|answer| {
                                !answer.selected_option_ids.is_empty()
                                    && (question.allow_multiple
                                        || answer.selected_option_ids.len() == 1)
                                    && answer
                                        .selected_option_ids
                                        .iter()
                                        .collect::<HashSet<_>>()
                                        .len()
                                        == answer.selected_option_ids.len()
                                    && answer.selected_option_ids.iter().all(|id| {
                                        question.options.iter().any(|option| &option.id == id)
                                    })
                            })
                });
            if !valid {
                return json!({"action": "cancel"});
            }
            let content = answers
                .into_iter()
                .map(|answer| {
                    let multiple = interaction.questions.iter().any(|question| {
                        question.id == answer.question_id && question.allow_multiple
                    });
                    let value = if multiple {
                        json!(answer.selected_option_ids)
                    } else {
                        json!(answer.selected_option_ids[0])
                    };
                    (answer.question_id, value)
                })
                .collect::<serde_json::Map<String, Value>>();
            json!({"action": "accept", "content": content})
        }
        (OperatorInteractionKind::Question, OperatorAnswer::Decline) => {
            json!({"action": "decline"})
        }
        (OperatorInteractionKind::Question, _) => json!({"action": "cancel"}),
        (OperatorInteractionKind::PlanApproval, OperatorAnswer::Plan { accepted: true }) => {
            json!({"outcome": {"outcome": "accepted"}})
        }
        (
            OperatorInteractionKind::PlanApproval,
            OperatorAnswer::Plan { accepted: false } | OperatorAnswer::Decline,
        ) => json!({"outcome": {"outcome": "rejected"}}),
        (OperatorInteractionKind::PlanApproval, _) => json!({"outcome": {"outcome": "cancelled"}}),
    }
}

/// Cursor's documented question result uses an `outcome` envelope instead of
/// standard ACP form elicitation's `action`/`content` result.
pub(super) fn response_for_method(
    method: &str,
    interaction: &OperatorInteraction,
    answer: OperatorAnswer,
) -> Value {
    let response = response_for(interaction, answer);
    if method != "cursor/ask_question" {
        return response;
    }
    match response.get("action").and_then(Value::as_str) {
        Some("accept") => {
            let answers = interaction
                .questions
                .iter()
                .map(|question| {
                    let selected = &response["content"][&question.id];
                    let selected_option_ids = selected
                        .as_array()
                        .cloned()
                        .unwrap_or_else(|| vec![selected.clone()]);
                    json!({"questionId": question.id, "selectedOptionIds": selected_option_ids})
                })
                .collect::<Vec<_>>();
            json!({"outcome":{"outcome":"answered","answers":answers}})
        }
        Some("decline") => json!({"outcome":{"outcome":"skipped"}}),
        _ => json!({"outcome":{"outcome":"cancelled"}}),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::opensymphony_gateway_schema::approval::OperatorQuestionAnswer;

    #[test]
    fn permission_uses_only_offered_option_and_keeps_peer_rpc_id_private() {
        let request = json!({"sessionId":"session-1","toolCall":{"toolCallId":"tool-1","title":"Run tests"},
            "options":[{"optionId":"opaque-allow","name":"Allow once","kind":"allow_once"},
                {"optionId":"opaque-deny","name":"Deny","kind":"reject_once"}]});
        let interaction = parse_interaction(
            "session/request_permission",
            &request,
            json!(0),
            "session-1",
            Duration::from_secs(30),
        )
        .expect("permission");
        assert!(uuid::Uuid::parse_str(&interaction.rpc_id).is_ok());
        assert_ne!(interaction.rpc_id, "0");
        let large_id = parse_interaction(
            "session/request_permission",
            &request,
            json!(9_007_199_254_740_993_u64),
            "session-1",
            Duration::from_secs(30),
        )
        .expect("large numeric RPC ID");
        assert!(uuid::Uuid::parse_str(&large_id.rpc_id).is_ok());
        assert_ne!(large_id.rpc_id, interaction.rpc_id);
        assert_eq!(
            serde_json::to_value(&large_id).expect("public binding")["rpc_id"],
            large_id.rpc_id
        );
        let sensitive_id = parse_interaction(
            "session/request_permission",
            &request,
            json!("credential-as-rpc-id"),
            "session-1",
            Duration::from_secs(30),
        )
        .expect("sensitive string ID");
        assert!(
            !serde_json::to_string(&sensitive_id)
                .expect("public interaction")
                .contains("credential-as-rpc-id")
        );
        assert_eq!(
            response_for(
                &interaction,
                OperatorAnswer::Permission {
                    option_id: "opaque-allow".into()
                }
            ),
            json!({"outcome":{"outcome":"selected","optionId":"opaque-allow"}})
        );
        assert_eq!(
            response_for(
                &interaction,
                OperatorAnswer::Permission {
                    option_id: "invented".into()
                }
            ),
            json!({"outcome":{"outcome":"cancelled"}})
        );
        assert_eq!(
            automatic_answer(AcpPermissionPolicy::Deny, &interaction),
            Some(OperatorAnswer::Permission {
                option_id: "opaque-deny".into()
            })
        );
        assert_eq!(
            automatic_answer(AcpPermissionPolicy::AllowOnce, &interaction),
            Some(OperatorAnswer::Permission {
                option_id: "opaque-allow".into()
            })
        );
        let mut no_matching_option = interaction.clone();
        no_matching_option
            .options
            .retain(|option| option.kind != "allow_once");
        assert_eq!(
            automatic_answer(AcpPermissionPolicy::AllowOnce, &no_matching_option),
            Some(OperatorAnswer::Cancel)
        );
        assert!(
            parse_interaction(
                "session/request_permission",
                &request,
                json!(0),
                "wrong-session",
                Duration::from_secs(30)
            )
            .is_err()
        );
    }

    #[test]
    fn structured_questions_accept_only_offered_complete_answers() {
        let request = json!({"sessionId":"s","mode":"form","message":"Choose deployment","requestedSchema":{
        "type":"object","required":["region","checks"],"properties":{
            "region":{"type":"string","title":"Region?","oneOf":[{"const":"east","title":"East"},{"const":"west","title":"West"}]},
            "checks":{"type":"array","title":"Checks?","items":{"type":"string","enum":["lint","test"]}}
        }}});
        let interaction = parse_interaction(
            "elicitation/create",
            &request,
            json!("rpc-2"),
            "s",
            Duration::from_secs(30),
        )
        .expect("question");
        let mut reordered = interaction.clone();
        reordered.questions.reverse();
        for question in &mut reordered.questions {
            question.options.reverse();
        }
        assert!(same_binding_ids(&interaction, &reordered));
        reordered.questions[0].options[0].id = "substituted-opaque-id".into();
        assert!(!same_binding_ids(&interaction, &reordered));
        let answer = OperatorAnswer::Question {
            answers: vec![
                OperatorQuestionAnswer {
                    question_id: "region".into(),
                    selected_option_ids: vec!["west".into()],
                },
                OperatorQuestionAnswer {
                    question_id: "checks".into(),
                    selected_option_ids: vec!["lint".into(), "test".into()],
                },
            ],
        };
        assert_eq!(
            response_for(&interaction, answer),
            json!({"action":"accept","content":{"region":"west","checks":["lint","test"]}})
        );
        let bad = OperatorAnswer::Question {
            answers: vec![
                OperatorQuestionAnswer {
                    question_id: "region".into(),
                    selected_option_ids: vec!["invented".into()],
                },
                OperatorQuestionAnswer {
                    question_id: "checks".into(),
                    selected_option_ids: vec!["lint".into()],
                },
            ],
        };
        assert_eq!(response_for(&interaction, bad)["action"], "cancel");
        assert_eq!(
            response_for(&interaction, OperatorAnswer::Decline)["action"],
            "decline"
        );
    }

    #[test]
    fn cursor_question_and_plan_use_vendor_response_contracts() {
        let question = parse_interaction(
            "cursor/ask_question",
            &json!({"sessionId":"s","toolCallId":"call-q","questions":[{"id":"mode","prompt":"Mode?","options":[{"id":"agent","label":"Agent"},{"id":"plan","label":"Plan"}]}],"_meta":{"traceparent":"trace"}}),
            json!(0), "s", Duration::from_secs(30),
        ).expect("Cursor question");
        let answered = response_for_method(
            "cursor/ask_question",
            &question,
            OperatorAnswer::Question {
                answers: vec![OperatorQuestionAnswer {
                    question_id: "mode".into(),
                    selected_option_ids: vec!["plan".into()],
                }],
            },
        );
        assert_eq!(
            answered,
            json!({"outcome":{"outcome":"answered","answers":[{"questionId":"mode","selectedOptionIds":["plan"]}]}})
        );
        assert_eq!(
            response_for_method("cursor/ask_question", &question, OperatorAnswer::Decline),
            json!({"outcome":{"outcome":"skipped"}})
        );
        assert_eq!(
            response_for_method("cursor/ask_question", &question, OperatorAnswer::Cancel),
            json!({"outcome":{"outcome":"cancelled"}})
        );
        assert!(
            parse_interaction(
                "cursor/ask_question",
                &json!({"sessionId":"other","toolCallId":"call-q","questions":[]}),
                json!(0),
                "s",
                Duration::from_secs(30)
            )
            .is_err()
        );

        let plan = parse_interaction(
            "cursor/create_plan",
            &json!({"sessionId":"s","toolCallId":"call-p","plan":"1. Verify","todos":[{"id":"t1","content":"Verify","status":"pending"}]}),
            json!(1), "s", Duration::from_secs(30),
        ).expect("Cursor plan");
        assert_eq!(plan.kind, OperatorInteractionKind::PlanApproval);
        assert_eq!(
            response_for_method(
                "cursor/create_plan",
                &plan,
                OperatorAnswer::Plan { accepted: true }
            ),
            json!({"outcome":{"outcome":"accepted"}})
        );
        let credential_plan = parse_interaction(
            "cursor/create_plan",
            &json!({"sessionId":"s","toolCallId":"call-credentials","name":"Rotate credentials","plan":"Validate token handling","todos":[{"id":"t1","content":"Update secret storage","status":"pending"}]}),
            json!(2), "s", Duration::from_secs(30),
        ).expect("plan text can discuss credential-related code");
        assert_eq!(credential_plan.title, "Rotate credentials");
    }

    #[test]
    fn plan_response_and_sensitive_question_boundary() {
        let mut interaction = parse_interaction(
            "session/request_permission",
            &json!({"sessionId":"s","toolCall":{"toolCallId":"t","title":"Ship"},
                "options":[{"optionId":"allow","name":"Allow once","kind":"allow_once"}]}),
            json!(7),
            "s",
            Duration::from_secs(30),
        )
        .expect("typed interaction");
        interaction.kind = OperatorInteractionKind::PlanApproval;
        interaction.options.clear();
        interaction.plan = Some("Run smoke tests".into());
        assert_eq!(
            response_for(&interaction, OperatorAnswer::Plan { accepted: true })["outcome"]["outcome"],
            "accepted"
        );
        assert_eq!(
            response_for(&interaction, OperatorAnswer::Cancel)["outcome"]["outcome"],
            "cancelled"
        );
        assert!(parse_interaction("elicitation/create", &json!({"sessionId":"s","mode":"form","message":"Enter API key","requestedSchema":{"type":"object","required":["key"],"properties":{"key":{"type":"string","enum":["a"]}}}}), json!(8), "s", Duration::from_secs(30)).is_err());
        assert!(parse_interaction("elicitation/create", &json!({"sessionId":"s","mode":"url","url":"https://example.com","elicitationId":"e"}), json!(8), "s", Duration::from_secs(30)).is_err());
        assert!(
            parse_interaction(
                "cursor/create_plan",
                &json!({"sessionId":"s","plan":"Ship"}),
                json!(9),
                "s",
                Duration::from_secs(30)
            )
            .is_err()
        );
        assert!(
            parse_interaction(
                "cursor/ask_question",
                &json!({"sessionId":"s","questions":[]}),
                json!(10),
                "s",
                Duration::from_secs(30)
            )
            .is_err()
        );
    }

    #[test]
    fn form_schema_rejects_free_text_optional_and_unsupported_constraints() {
        let base = json!({"sessionId":"s","mode":"form","message":"Choose a region",
            "requestedSchema":{"type":"object","required":["region"],"properties":{
                "region":{"type":"string","enum":["east","west"]}}}});
        for property in [
            json!({"type":"string"}),
            json!({"type":"number"}),
            json!({"type":"string","enum":["east","west"],"pattern":"^east$"}),
            json!({"type":"string","enum":["east","east"]}),
        ] {
            let mut request = base.clone();
            request["requestedSchema"]["properties"]["region"] = property;
            assert!(
                parse_interaction(
                    "elicitation/create",
                    &request,
                    json!(0),
                    "s",
                    Duration::from_secs(30)
                )
                .is_err()
            );
        }
        let mut optional = base.clone();
        optional["requestedSchema"]["required"] = json!([]);
        assert!(
            parse_interaction(
                "elicitation/create",
                &optional,
                json!(0),
                "s",
                Duration::from_secs(30)
            )
            .is_err()
        );
        assert!(
            parse_interaction(
                "elicitation/create",
                &base,
                json!(0),
                "other",
                Duration::from_secs(30)
            )
            .is_err()
        );
    }
}
