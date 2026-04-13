#[path = "support/mod.rs"]
mod compat;
pub use compat::*;

#[path = "../crates/opensymphony-openhands/tests/live_local_suite.rs"]
mod live_local_suite;
