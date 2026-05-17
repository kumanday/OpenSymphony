#[path = "support/mod.rs"]
mod compat;
pub use compat::*;

#[path = "../crates/opensymphony-gateway-schema/tests/gateway_schema.rs"]
mod gateway_schema;
