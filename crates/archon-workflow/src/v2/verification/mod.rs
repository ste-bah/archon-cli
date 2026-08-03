//! Focused-verification outcome adjudication.
//!
//! Everything here decides whether a verification branch's self-report may be
//! believed. A verifier reports its own `commands_run`, its own status and its
//! own coverage, so the host re-reads that report against evidence it can check
//! itself: did any command actually succeed, did the filters it named match any
//! tests, and — when the item declared a deliverable contract — does the
//! contract still hold when the HOST runs the verifier rather than the audited
//! branch. Each check fails closed; "we could not check" is never a pass.
//!
//! The failure side is classified rather than merely rejected, because the
//! lifecycle has to know whether to retry the verification, route the work to
//! remediation, or stop.
//!
//! It sits in this crate rather than the binary because every input and output
//! is a type this crate owns; nothing here touches CLI state, config layering,
//! or the terminal.

mod failure_class;
mod normalize;
mod signals;

pub use normalize::{
    enforce_declared_contracts, normalize_focused_verification_outcome,
    stamp_focused_verification_input,
};
