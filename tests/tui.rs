#[path = "support/mod.rs"]
mod compat;
pub use compat::*;

#[path = "../crates/opensymphony-cli/tests/tui.rs"]
mod tui;
