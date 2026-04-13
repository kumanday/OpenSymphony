#[path = "support/mod.rs"]
mod compat;
pub use compat::*;

#[path = "../crates/opensymphony-tui/tests/reducer.rs"]
mod reducer;
