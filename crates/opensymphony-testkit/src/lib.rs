//! Deterministic fake OpenHands server and contract-focused fixtures.

mod fake_openhands;

pub use fake_openhands::{
    ConversationRecord, FakeOpenHandsServer, RunStep, ScriptedRun, TestkitError,
    execution_status_event, full_state_event, generic_event,
};
