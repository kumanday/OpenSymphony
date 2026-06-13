use std::collections::HashSet;

use serde_json::json;

use super::client::LinearClient;
use super::error::LinearError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequiredField {
    pub type_name: &'static str,
    pub field_name: &'static str,
    pub critical: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchemaDriftReport {
    pub is_compatible: bool,
    pub missing_fields: Vec<SchemaDriftViolation>,
    pub checked_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchemaDriftViolation {
    pub type_name: String,
    pub field_name: String,
    pub critical: bool,
    pub remediation: String,
}

#[derive(Debug, serde::Deserialize)]
pub struct IntrospectedType {
    #[allow(dead_code)]
    pub kind: String,
    #[allow(dead_code)]
    pub name: Option<String>,
    #[serde(default)]
    pub fields: Option<Vec<IntrospectedField>>,
}

#[derive(Debug, serde::Deserialize)]
pub struct IntrospectedField {
    pub name: String,
}

/// Canonical list of required Linear GraphQL fields.
///
/// # Remediation
/// 1. Check the Linear API changelog for the breaking change.
/// 2. Update the corresponding GraphQL query constant in `graphql.rs`.
/// 3. Update the Rust model in `graphql.rs` / `normalize.rs` to match.
/// 4. If the field is no longer needed, remove it from this list.
pub fn required_fields() -> &'static [RequiredField] {
    &[
        RequiredField {
            type_name: "Issue",
            field_name: "id",
            critical: true,
        },
        RequiredField {
            type_name: "Issue",
            field_name: "identifier",
            critical: true,
        },
        RequiredField {
            type_name: "Issue",
            field_name: "url",
            critical: true,
        },
        RequiredField {
            type_name: "Issue",
            field_name: "title",
            critical: true,
        },
        RequiredField {
            type_name: "Issue",
            field_name: "description",
            critical: false,
        },
        RequiredField {
            type_name: "Issue",
            field_name: "priority",
            critical: false,
        },
        RequiredField {
            type_name: "Issue",
            field_name: "createdAt",
            critical: true,
        },
        RequiredField {
            type_name: "Issue",
            field_name: "updatedAt",
            critical: true,
        },
        RequiredField {
            type_name: "Issue",
            field_name: "state",
            critical: true,
        },
        RequiredField {
            type_name: "Issue",
            field_name: "parent",
            critical: false,
        },
        RequiredField {
            type_name: "Issue",
            field_name: "projectMilestone",
            critical: false,
        },
        RequiredField {
            type_name: "Issue",
            field_name: "children",
            critical: false,
        },
        RequiredField {
            type_name: "Issue",
            field_name: "labels",
            critical: false,
        },
        RequiredField {
            type_name: "Issue",
            field_name: "inverseRelations",
            critical: true,
        },
        RequiredField {
            type_name: "Issue",
            field_name: "comments",
            critical: false,
        },
        RequiredField {
            type_name: "WorkflowState",
            field_name: "id",
            critical: true,
        },
        RequiredField {
            type_name: "WorkflowState",
            field_name: "name",
            critical: true,
        },
        RequiredField {
            type_name: "WorkflowState",
            field_name: "type",
            critical: true,
        },
        RequiredField {
            type_name: "Project",
            field_name: "id",
            critical: true,
        },
        RequiredField {
            type_name: "Project",
            field_name: "name",
            critical: true,
        },
        RequiredField {
            type_name: "Project",
            field_name: "slugId",
            critical: true,
        },
        RequiredField {
            type_name: "Project",
            field_name: "url",
            critical: false,
        },
        RequiredField {
            type_name: "Project",
            field_name: "content",
            critical: false,
        },
        RequiredField {
            type_name: "ProjectMilestone",
            field_name: "id",
            critical: true,
        },
        RequiredField {
            type_name: "ProjectMilestone",
            field_name: "name",
            critical: true,
        },
        RequiredField {
            type_name: "Label",
            field_name: "id",
            critical: true,
        },
        RequiredField {
            type_name: "Label",
            field_name: "name",
            critical: true,
        },
        RequiredField {
            type_name: "Comment",
            field_name: "id",
            critical: true,
        },
        RequiredField {
            type_name: "Comment",
            field_name: "body",
            critical: true,
        },
        RequiredField {
            type_name: "Comment",
            field_name: "updatedAt",
            critical: false,
        },
        RequiredField {
            type_name: "Comment",
            field_name: "resolvedAt",
            critical: false,
        },
        RequiredField {
            type_name: "ProjectMilestoneCreateInput",
            field_name: "projectId",
            critical: true,
        },
        RequiredField {
            type_name: "ProjectMilestoneCreateInput",
            field_name: "name",
            critical: true,
        },
        RequiredField {
            type_name: "IssueCreateInput",
            field_name: "teamId",
            critical: true,
        },
        RequiredField {
            type_name: "IssueCreateInput",
            field_name: "title",
            critical: true,
        },
        RequiredField {
            type_name: "IssueRelationCreateInput",
            field_name: "issueId",
            critical: true,
        },
        RequiredField {
            type_name: "IssueRelationCreateInput",
            field_name: "relatedIssueId",
            critical: true,
        },
        RequiredField {
            type_name: "IssueRelationCreateInput",
            field_name: "type",
            critical: true,
        },
        RequiredField {
            type_name: "CommentCreateInput",
            field_name: "issueId",
            critical: true,
        },
        RequiredField {
            type_name: "CommentCreateInput",
            field_name: "body",
            critical: true,
        },
    ]
}

pub(super) const INTROSPECT_TYPE_QUERY: &str = r#"
query IntrospectType($typeName: String!) {
  __type(name: $typeName) {
    kind
    name
    fields(includeDeprecated: true) {
      name
    }
  }
}
"#;

impl LinearClient {
    /// Run schema drift validation against the live Linear API.
    ///
    /// Returns a report indicating whether the current schema requirements
    /// are satisfied by the remote API.
    pub async fn check_schema_drift(&self) -> Result<SchemaDriftReport, LinearError> {
        let required = required_fields();
        let mut missing = Vec::new();
        let type_names: HashSet<&str> = required.iter().map(|f| f.type_name).collect();
        let mut remote_fields: std::collections::HashMap<String, HashSet<String>> =
            std::collections::HashMap::new();

        for type_name in &type_names {
            let fields = self.introspect_type(type_name).await?;
            remote_fields.insert(type_name.to_string(), fields.into_iter().collect());
        }

        let checked_at = Some(chrono::Utc::now());

        for req in required {
            let field_set = remote_fields.get(req.type_name);
            match field_set {
                Some(set) if set.contains(req.field_name) => {}
                _ => {
                    missing.push(SchemaDriftViolation {
                        type_name: req.type_name.to_string(),
                        field_name: req.field_name.to_string(),
                        critical: req.critical,
                        remediation: format!(
                            "Field `{}` on type `{}` missing in remote Linear schema. Check Linear API changelog, update graphql.rs / normalize.rs, or remove from required_fields().",
                            req.field_name, req.type_name
                        ),
                    });
                }
            }
        }

        Ok(SchemaDriftReport {
            is_compatible: missing.is_empty(),
            missing_fields: missing,
            checked_at,
        })
    }

    pub(super) async fn introspect_type(
        &self,
        type_name: &str,
    ) -> Result<Vec<String>, LinearError> {
        let response: serde_json::Value = self
            .execute_graphql(INTROSPECT_TYPE_QUERY, json!({ "typeName": type_name }))
            .await?;

        let type_node = response.get("data").and_then(|d| d.get("__type"));
        match type_node {
            None | Some(serde_json::Value::Null) => {
                return Err(LinearError::InvalidResponse(format!(
                    "Introspection returned null for type `{type_name}` (type may not exist or was renamed)"
                )));
            }
            _ => {}
        }

        let fields = type_node
            .and_then(|t| t.get("fields"))
            .and_then(|f| f.as_array())
            .ok_or_else(|| {
                LinearError::InvalidResponse(format!(
                    "Introspection for type `{type_name}` returned no fields"
                ))
            })?;

        let mut names = Vec::new();
        for field in fields {
            if let Some(name) = field.get("name").and_then(|n| n.as_str()) {
                names.push(name.to_string());
            }
        }
        Ok(names)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn required_fields_contains_issue_core_fields() {
        let fields = required_fields();
        let issue_fields: Vec<_> = fields
            .iter()
            .filter(|f| f.type_name == "Issue")
            .map(|f| f.field_name)
            .collect();
        assert!(issue_fields.contains(&"id"));
        assert!(issue_fields.contains(&"identifier"));
        assert!(issue_fields.contains(&"state"));
        assert!(issue_fields.contains(&"inverseRelations"));
    }

    #[test]
    fn required_fields_marks_id_as_critical() {
        for field in required_fields() {
            if field.field_name == "id" {
                assert!(
                    field.critical,
                    "id field on {} must be critical",
                    field.type_name
                );
            }
        }
    }

    #[test]
    fn schema_drift_report_compatible_when_no_violations() {
        let report = SchemaDriftReport {
            is_compatible: true,
            missing_fields: Vec::new(),
            checked_at: None,
        };
        assert!(report.is_compatible);
        assert!(report.missing_fields.is_empty());
    }

    #[test]
    fn schema_drift_report_incompatible_with_violations() {
        let report = SchemaDriftReport {
            is_compatible: false,
            missing_fields: vec![SchemaDriftViolation {
                type_name: "Issue".to_string(),
                field_name: "deletedField".to_string(),
                critical: true,
                remediation: "Remove from query".to_string(),
            }],
            checked_at: None,
        };
        assert!(!report.is_compatible);
        assert_eq!(report.missing_fields.len(), 1);
        assert!(report.missing_fields[0].critical);
    }
}
