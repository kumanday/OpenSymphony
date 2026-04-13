#[path = "support/mod.rs"]
mod compat;
pub use compat::*;

#[path = "../crates/opensymphony-domain/tests/snapshot_serialization.rs"]
mod snapshot_serialization;
