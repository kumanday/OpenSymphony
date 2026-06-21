#[path = "../crates/opensymphony-cli/src/lib.rs"]
pub mod opensymphony_cli;
#[path = "../crates/opensymphony-codex/src/lib.rs"]
pub mod opensymphony_codex;
#[path = "../crates/opensymphony-control/src/lib.rs"]
pub mod opensymphony_control;
#[path = "../crates/opensymphony-domain/src/lib.rs"]
pub mod opensymphony_domain;
#[path = "../crates/opensymphony-gateway/src/lib.rs"]
pub mod opensymphony_gateway;
#[path = "../crates/opensymphony-gateway-schema/src/lib.rs"]
pub mod opensymphony_gateway_schema;
#[path = "../crates/opensymphony-linear/src/lib.rs"]
pub mod opensymphony_linear;
#[path = "../crates/opensymphony-memory/src/lib.rs"]
pub mod opensymphony_memory;
#[path = "../crates/opensymphony-openhands/src/lib.rs"]
pub mod opensymphony_openhands;
#[path = "../crates/opensymphony-orchestrator/src/lib.rs"]
pub mod opensymphony_orchestrator;
#[path = "../crates/opensymphony-planning/src/lib.rs"]
pub mod opensymphony_planning;
#[path = "../crates/opensymphony-testkit/src/lib.rs"]
pub mod opensymphony_testkit;
#[path = "../crates/opensymphony-tui/src/lib.rs"]
pub mod opensymphony_tui;
#[path = "../crates/opensymphony-workflow/src/lib.rs"]
pub mod opensymphony_workflow;
#[path = "../crates/opensymphony-workspace/src/lib.rs"]
pub mod opensymphony_workspace;

pub use crate::opensymphony_cli::run;
// Re-export the gateway task-graph mutation types so the integration tests
// under `crates/opensymphony-gateway/tests/` and any external consumer can
// import them straight from the gateway module instead of having to know
// the internal module layout.
pub use crate::opensymphony_gateway::task_graph_mutations::{
    IssueOp, LinearClientMutationAdapter, LinearMutationClient, MilestoneOp, MutationError,
    SubIssueOp, TaskGraphEvidenceRequest, TaskGraphEvidenceResponse, TaskGraphIssueRequest,
    TaskGraphIssueResponse, TaskGraphMilestoneRequest, TaskGraphMilestoneResponse,
    TaskGraphMutationState, TaskGraphRelationRequest, TaskGraphRelationResponse,
    TaskGraphSubIssueRequest, TaskGraphSubIssueResponse, append_mutation_event, entity_kind_for,
    task_graph_router,
};
