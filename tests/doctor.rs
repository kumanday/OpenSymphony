#[path = "support/mod.rs"]
mod compat;
pub use compat::*;

#[path = "../crates/opensymphony-cli/tests/doctor.rs"]
mod doctor;
