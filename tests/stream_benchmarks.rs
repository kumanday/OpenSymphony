#[path = "support/mod.rs"]
mod compat;
pub use compat::*;

#[path = "../crates/opensymphony-gateway-schema/tests/stream_benchmarks.rs"]
mod stream_benchmarks;
