//! The review band: pairs distance could not settle, and the verdicts on them.

use crate::access::MemoryTrait;
use crate::types::MemoryError;
use serde::{Deserialize, Serialize};

/// Two memories close enough to be about the same thing, but not close enough
/// for distance alone to prove they make the same claim.
///
/// Returned rather than acted on. `archon-memory` is a leaf crate with no
/// provider access, and consolidation is synchronous, so the judgement is made
/// by the caller -- which has both -- and applied with
/// [`apply_adjudicated_merges`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewPair {
    pub a_id: String,
    pub b_id: String,
    pub a_content: String,
    pub b_content: String,
}

/// A caller's verdict on a [`ReviewPair`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Adjudication {
    /// The same claim in different words. Merge them.
    SameClaim,
    /// Related but distinct. Leave both.
    Distinct,
}

/// Apply verdicts from an adjudicator, merging only the `SameClaim` pairs.
///
/// Returns how many merges were performed. A pair whose memories have since been
/// removed or already superseded is skipped rather than failing the batch: the
/// verdicts are made against a snapshot and the store keeps moving.
pub fn apply_adjudicated_merges(
    graph: &dyn MemoryTrait,
    verdicts: &[(ReviewPair, Adjudication)],
) -> Result<usize, MemoryError> {
    super::phases::apply_adjudicated_merges(graph, verdicts)
}
