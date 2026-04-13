#[path = "support/mod.rs"]
mod compat;
pub use compat::*;

#[path = "../crates/opensymphony-openhands/tests/fake_server_contract.rs"]
mod fake_server_contract;
