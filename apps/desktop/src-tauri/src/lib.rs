//! OpenSymphony Tauri desktop shell library.
//!
//! Exports Tauri commands that are explicitly scoped via capability files.
//! Each command uses narrow request and response types to limit attack surface.

pub mod commands;
