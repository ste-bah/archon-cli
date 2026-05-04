//! Generic provenance substrate for the Archon Evidence Engine.

pub mod chain;
pub mod errors;
pub mod export_w3c;
pub mod record;
pub mod store;
pub mod traverse;
pub mod verify;

pub use errors::{ProvenanceError, Result};
pub use record::{ProvenanceEdge, ProvenanceEdgeType, ProvenanceRecord};
