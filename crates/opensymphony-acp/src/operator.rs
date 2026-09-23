//! Bounded, typed operator callbacks. No arbitrary ACP method passthrough.
use super::*;
use crate::opensymphony_gateway_schema::approval::{
    OperatorAnswer, OperatorInteraction, OperatorInteractionKind, OperatorOption, OperatorQuestion,
};
use crate::opensymphony_workflow::AcpPermissionPolicy;
use agent_client_protocol::schema::v1::RequestPermissionRequest;
use chrono::{Duration as ChronoDuration, Utc};
use std::collections::HashSet;

pub struct AcpOperatorRequest {
    pub interaction: OperatorInteraction,
    pub reply: oneshot::Sender<AcpOperatorReply>,
}

pub struct AcpOperatorReply {
    pub answer: OperatorAnswer,
    pub acknowledgement: oneshot::Sender<bool>,
}

pub enum AcpOperatorEvent {
    Opened(Box<AcpOperatorRequest>),
    Closed(String),
}

pub(super) type OperatorRouter =
    Arc<Mutex<Option<(mpsc::Sender<AcpOperatorEvent>, CancellationToken)>>>;

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
        "cursor/ask_question" => {
            let raw = params.as_object().ok_or_else(invalid)?;
            if raw.get("sessionId").and_then(Value::as_str) != Some(session_id) {
                return Err(invalid());
            }
            let questions_raw = raw
                .get("questions")
                .and_then(Value::as_array)
                .ok_or_else(invalid)?;
            if questions_raw.is_empty() || questions_raw.len() > 8 {
                return Err(invalid());
            }
            let mut questions = Vec::with_capacity(questions_raw.len());
            let mut question_ids = HashSet::new();
            for raw in questions_raw {
                let id = raw.get("id").and_then(Value::as_str).ok_or_else(invalid)?;
                let prompt = raw
                    .get("prompt")
                    .and_then(Value::as_str)
                    .ok_or_else(invalid)?;
                let choices = raw
                    .get("options")
                    .and_then(Value::as_array)
                    .ok_or_else(invalid)?;
                if !bounded(id, 128)
                    || !bounded(prompt, 2048)
                    || !no_secret_prompt(prompt)
                    || !question_ids.insert(id)
                    || choices.is_empty()
                    || choices.len() > 32
                {
                    return Err(invalid());
                }
                let mut option_ids = HashSet::new();
                let mut options = Vec::with_capacity(choices.len());
                for choice in choices {
                    let id = choice
                        .get("id")
                        .and_then(Value::as_str)
                        .ok_or_else(invalid)?;
                    let label = choice
                        .get("label")
                        .and_then(Value::as_str)
                        .ok_or_else(invalid)?;
                    if !bounded(id, 128)
                        || !bounded(label, 1024)
                        || !no_secret_prompt(label)
                        || !option_ids.insert(id)
                    {
                        return Err(invalid());
                    }
                    options.push(OperatorOption {
                        id: id.into(),
                        label: label.into(),
                        kind: "choice".into(),
                    });
                }
                questions.push(OperatorQuestion {
                    id: id.into(),
                    prompt: prompt.into(),
                    options,
                    allow_multiple: raw
                        .get("allowMultiple")
                        .and_then(Value::as_bool)
                        .unwrap_or(false),
                });
            }
            let title = raw
                .get("title")
                .and_then(Value::as_str)
                .unwrap_or("ACP question")
                .to_owned();
            (
                OperatorInteractionKind::Question,
                title,
                Vec::new(),
                questions,
                None,
            )
        }
        "cursor/create_plan" => {
            let raw = params.as_object().ok_or_else(invalid)?;
            if raw.get("sessionId").and_then(Value::as_str) != Some(session_id) {
                return Err(invalid());
            }
            let plan = raw
                .get("plan")
                .and_then(Value::as_str)
                .ok_or_else(invalid)?;
            if !bounded(plan, 32 * 1024) || !no_secret_prompt(plan) {
                return Err(invalid());
            }
            let title = raw
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("ACP plan approval")
                .to_owned();
            (
                OperatorInteractionKind::PlanApproval,
                title,
                Vec::new(),
                Vec::new(),
                Some(plan.to_owned()),
            )
        }
        _ => return Err(agent_client_protocol::Error::method_not_found()),
    };
    if !bounded(&title, 2048)
        || !no_secret_prompt(&title)
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
        rpc_id,
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
            if valid {
                json!({"outcome": {"outcome": "answered", "answers": answers.iter().map(|answer| json!({"questionId": answer.question_id, "selectedOptionIds": answer.selected_option_ids})).collect::<Vec<_>>()}})
            } else {
                json!({"outcome": {"outcome": "cancelled"}})
            }
        }
        (OperatorInteractionKind::Question, OperatorAnswer::Decline) => {
            json!({"outcome": {"outcome": "skipped"}})
        }
        (OperatorInteractionKind::Question, _) => json!({"outcome": {"outcome": "cancelled"}}),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::opensymphony_gateway_schema::approval::OperatorQuestionAnswer;

    #[test]
    fn permission_uses_only_offered_opaque_option_and_preserves_zero_rpc_id() {
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
        assert_eq!(interaction.rpc_id, json!(0));
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
        let request = json!({"sessionId":"s","title":"Choose deployment","questions":[
            {"id":"region","prompt":"Region?","options":[{"id":"east","label":"East"},{"id":"west","label":"West"}]},
            {"id":"checks","prompt":"Checks?","allowMultiple":true,"options":[{"id":"lint","label":"Lint"},{"id":"test","label":"Test"}]}]});
        let interaction = parse_interaction(
            "cursor/ask_question",
            &request,
            json!("rpc-2"),
            "s",
            Duration::from_secs(30),
        )
        .expect("question");
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
            response_for(&interaction, answer)["outcome"]["outcome"],
            "answered"
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
        assert_eq!(
            response_for(&interaction, bad)["outcome"]["outcome"],
            "cancelled"
        );
        assert_eq!(
            response_for(&interaction, OperatorAnswer::Decline)["outcome"]["outcome"],
            "skipped"
        );
    }

    #[test]
    fn plan_response_and_sensitive_question_boundary() {
        let interaction = parse_interaction(
            "cursor/create_plan",
            &json!({"sessionId":"s","name":"Ship","plan":"Run smoke tests"}),
            json!(7),
            "s",
            Duration::from_secs(30),
        )
        .expect("plan");
        assert_eq!(
            response_for(&interaction, OperatorAnswer::Plan { accepted: true })["outcome"]["outcome"],
            "accepted"
        );
        assert_eq!(
            response_for(&interaction, OperatorAnswer::Cancel)["outcome"]["outcome"],
            "cancelled"
        );
        assert!(parse_interaction("cursor/ask_question", &json!({"sessionId":"s","questions":[{"id":"x","prompt":"Enter API key","options":[{"id":"a","label":"A"}]}]}), json!(8), "s", Duration::from_secs(30)).is_err());
    }
}
