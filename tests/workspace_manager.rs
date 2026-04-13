#[path = "support/mod.rs"]
mod compat;
pub use compat::*;

#[path = "../crates/opensymphony-workspace/tests/workspace_manager.rs"]
mod workspace_manager;
