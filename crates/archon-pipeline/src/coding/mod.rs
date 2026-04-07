//! Coding pipeline (48 agents).

pub mod agents;
pub mod algorithm;
pub mod contract;
pub mod evidence;
pub mod facade;
pub mod gates;
pub mod quality;
pub mod rlm;
pub mod hooks;
pub mod wiring;

pub use agents::{Algorithm, CodingAgent, Phase, ToolAccess, AGENTS};
