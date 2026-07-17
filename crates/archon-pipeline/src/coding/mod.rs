//! Coding pipeline (50 agents).

pub mod agents;
pub mod algorithm;
mod compilation_gate;
pub mod contract;
pub mod evidence;
pub mod facade;
pub mod gates;
pub mod hooks;
mod orphan_gate;
#[cfg(test)]
mod orphan_gate_go_import_tests;
#[cfg(test)]
mod orphan_gate_python_import_tests;
#[cfg(test)]
mod orphan_gate_tests;
pub mod quality;
pub mod rlm;
pub mod wiring;

pub use agents::{AGENTS, Algorithm, CodingAgent, Phase, ToolAccess};
