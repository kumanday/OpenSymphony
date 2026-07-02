#[path = "support/mod.rs"]
mod compat;
pub use compat::*;

#[path = "../crates/opensymphony-jira/tests/jira_client.rs"]
mod jira_client;
