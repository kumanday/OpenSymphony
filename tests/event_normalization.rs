#[path = "support/mod.rs"]
mod compat;
pub use compat::*;

#[path = "../crates/opensymphony-openhands/tests/event_normalization.rs"]
mod event_normalization;
