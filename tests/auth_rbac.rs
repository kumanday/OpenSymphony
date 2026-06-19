#[path = "support/mod.rs"]
mod compat;
pub use compat::*;

#[path = "../crates/opensymphony-gateway/tests/auth_rbac.rs"]
mod auth_rbac;
