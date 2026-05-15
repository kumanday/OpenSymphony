#[path = "support/mod.rs"]
mod compat;
pub use compat::*;

#[path = "../crates/opensymphony-cli/tests/memory.rs"]
mod memory;
