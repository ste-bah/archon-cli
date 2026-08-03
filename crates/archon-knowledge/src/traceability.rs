//! Requirement → code traceability, with a proof ladder that an unproven edge
//! can never climb by accident.
//!
//! # Why this exists
//!
//! A real run produced a gap report claiming all 93 PRD requirements were
//! mapped to code. Its own adversarial reviewer falsified it — finding F1:
//! *"Accepted verification treats 170 unique IDs as satisfying normative
//! requirement mapping… artifact sample has repeated generic evidence for
//! REQ-DL-001..004."* The mapping was padded. The same generic sentence was
//! reused across many requirements, and prose has no structure that can refuse
//! to be reused.
//!
//! A graph with anchored edges makes that padding structurally impossible: an
//! edge either names a `file:line` that exists on disk with a recorded content
//! hash, or the edge does not exist. There is nowhere for a generic sentence to
//! live.
//!
//! # The ladder
//!
//! [`ladder::ProofLevel`] is ordered, and only the top two count:
//!
//! - `Unproven` — the fail-closed floor. No live anchor: none was found, or
//!   every anchor's file changed since it was recorded. **Never satisfies a
//!   promotion gate.**
//! - `Candidate` — a semantic-search anchor exists and its file still hashes to
//!   what was recorded. Cheap, and *exactly what F1 mistook for proof*. It is a
//!   citation to go verify, never a verdict. **Never satisfies a promotion
//!   gate.**
//! - `Exercised` — a verifier command the task itself named PASSED, and the
//!   ambient trace shows that run read the anchored file. This is the level
//!   that kills F1: one command's trace cannot touch four unrelated anchors, so
//!   generic evidence repeated across `REQ-DL-001..004` cannot promote four
//!   requirements at once.
//! - `Falsifiable` — breaking the anchored code breaks the verifier. Planned
//!   here ([`falsification`]), never executed here, and an unexecuted plan
//!   promotes nothing.
//!
//! The rule that an unproven edge never satisfies a gate is the same rule
//! REQ-BT-003 already applies to diagnostic overrides.
//!
//! # What is deliberately absent
//!
//! No relevance threshold decides satisfaction, and no learned weight goes
//! anywhere near this module. Semantic search returns *candidates*; a score of
//! 0.97 and a score of 0.31 are both `Candidate` and neither is evidence.
//! Letting a number stand in for a trace would reproduce F1 with better maths
//! behind it, which is the one failure mode this module exists to prevent.
//!
//! # Why `scan_claims` is not wired in
//!
//! [`crate::contradiction_scanner::scan_claims`] was the obvious reuse, and it
//! does not fit. It pairs [`crate::schema::ClaimRecord`]s with equal
//! `normalized_subject` and `normalized_predicate` and opposite polarity —
//! a contradiction between two *assertions*. An anchor edge has no polarity: two
//! anchors for one requirement are corroboration, not disagreement, and there is
//! no negative form of "this code is about this requirement" for them to
//! contradict. Forcing requirements through the claim shape would produce
//! polarity fields that mean nothing and contradictions that are artefacts of
//! the encoding.
//!
//! The contradiction that actually matters here is F1's, and it is a different
//! shape: one span standing in as evidence for many requirements.
//! [`report::find_shared_anchors`] computes it directly from the graph.
//!
//! # Indexing runs out of band
//!
//! Nothing here indexes anything. [`anchors::CodeSearch`] is a read-only port;
//! the `archon-leann` adapter that implements it in the command layer opens the
//! code index without creating a schema and without a write. `archon-leann`'s
//! `replace_file_with_cancel`/`remove_file` hold the Cozo write lock across an
//! entire `multi_transaction` — the longest critical section in the repository
//! — so indexing mid-report would contend with every concurrent workflow.

pub mod anchors;
pub mod coverage;
pub mod falsification;
pub mod ladder;
pub mod report;
pub mod requirements;
pub mod store;
pub mod tasks;

pub use anchors::{Anchor, AnchorFreshness, CodeHit, CodeSearch};
pub use coverage::{CoverageReport, PhantomCitation};
pub use falsification::{FalsificationPlan, MutationKind};
pub use ladder::{
    CommandEvidence, ExercisedProof, MissingForPromotion, ProofLevel, ReadEvidence, ReadScope,
};
pub use report::{RequirementRow, TraceReport};
pub use requirements::{Requirement, Severity};
pub use tasks::{FocusedTestEntry, TaskBinding, VerifierCommand, VerifierOrigin};
