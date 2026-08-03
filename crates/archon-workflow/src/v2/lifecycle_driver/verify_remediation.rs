// Verification-failure triage and remediation helpers for the native
// decomposed-PRD lifecycle.

use super::*;

#[path = "verify_remediation_a.rs"]
mod verify_remediation_a;
/// Re-exported through the module chain because the binary's terminal-status
/// accounting keys on the same transport detector this retry path does.
pub use verify_remediation_a::is_transport_failure_text;
pub(crate) use verify_remediation_a::*;
#[path = "verify_remediation_b.rs"]
mod verify_remediation_b;
pub(crate) use verify_remediation_b::*;
