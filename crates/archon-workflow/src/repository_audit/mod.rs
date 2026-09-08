//! Snapshot-bound semantic audit records. These records never grant write authority.
pub mod contract;
pub use contract::{AuditContract, AuditRecord, AuditReport, RequiredAction, Verdict};

pub mod budget;

pub mod ledger;

pub mod runtime;

pub mod snapshot;

pub mod reuse;

mod status;

mod changes;

pub mod correction;
