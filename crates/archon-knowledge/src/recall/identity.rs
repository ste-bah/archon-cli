//! Dedupe by identity, and surface disagreement instead of resolving it.
//!
//! # Two different jobs that look like one
//!
//! Four stores over one corpus return the same fact more than once — the same
//! chunk reached through docs and through the knowledge graph is one artifact,
//! not two results, and letting both through spends the caller's limit twice on
//! one piece of evidence. That is dedupe, and it is safe because the two hits
//! *say the same thing*.
//!
//! Two hits that name the same provenance and say **different** things are the
//! opposite case, and merging them is the failure mode this module exists to
//! prevent. Picking the higher-scored one would delete the disagreement, and
//! the score doing the picking is [uncalibrated]. So conflicting hits are all
//! retained, and the disagreement is reported as a first-class part of the
//! answer.
//!
//! # The two conflict rules
//!
//! - [`ConflictKind::DivergentContentForProvenance`] — one provenance
//!   reference, more than one distinct content identity. Something claims to be
//!   the same artifact while carrying different text: a stale index, a rewritten
//!   chunk, or two stores that disagree about what the artifact says.
//! - [`ConflictKind::OppositePolarity`] — the same normalized subject and
//!   predicate asserted positively by one hit and negatively by another. This
//!   reuses the crate's existing claim parser and contradiction predicate rather
//!   than inventing a second, differently-behaved notion of "contradiction".
//!
//! Both rules are deterministic and syntactic. Neither is a semantic
//! entailment check, and neither should be read as one: a conflict here is a
//! flag for a caller to adjudicate, exactly like a traceability anchor is a
//! citation to go verify.
//!
//! [uncalibrated]: super::normalize

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::contradiction_scanner::contradicts;
use crate::recall::{HitRef, RecallHit, RecallSource};
use crate::schema::ClaimRecord;
use crate::store::DocumentChunk;
use crate::{claim_extractor, stable_id};

/// Why two or more hits are reported as disagreeing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConflictKind {
    /// One provenance reference, several distinct contents.
    DivergentContentForProvenance,
    /// Same subject and predicate, opposite polarity.
    OppositePolarity,
}

impl ConflictKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ConflictKind::DivergentContentForProvenance => "divergent_content_for_provenance",
            ConflictKind::OppositePolarity => "opposite_polarity",
        }
    }
}

/// One side of a disagreement, carried in full so the conflict survives the
/// truncation of the hit list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConflictMember {
    pub source: RecallSource,
    pub source_id: String,
    pub content: String,
}

/// A disagreement between hits, reported rather than resolved.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecallConflict {
    pub kind: ConflictKind,
    /// What the members disagree *about*: a provenance reference, or a
    /// `subject / predicate` pair.
    pub identity: String,
    pub members: Vec<ConflictMember>,
    /// One line a human can read without opening the members.
    pub explanation: String,
}

/// Deduped hits plus every disagreement found among them.
#[derive(Debug, Clone, Default)]
pub struct Merged {
    pub hits: Vec<RecallHit>,
    pub conflicts: Vec<RecallConflict>,
}

/// Identity of a piece of content, stable across stores.
///
/// Case, surrounding whitespace and trailing punctuation are dropped, because
/// those are artefacts of how each store chunked and stored the text rather than
/// differences in what it says. Internal punctuation is kept: "is safe" and "is
/// not safe" must never collide, and any normalisation aggressive enough to fuse
/// formatting differences starts risking that.
pub fn content_identity(content: &str) -> String {
    let collapsed = content.split_whitespace().collect::<Vec<_>>().join(" ");
    let trimmed = collapsed
        .trim_end_matches(|c: char| c.is_ascii_punctuation())
        .to_lowercase();
    stable_id("content", &[&trimmed])
}

/// Deterministic merge order: best score first, then the source enum order,
/// then the store's own id. No two distinct hits can tie, so the winner of a
/// dedupe group and the order of the final list are the same on every replay.
fn merge_order(hit: &RecallHit) -> (std::cmp::Reverse<ordered_score::Key>, RecallSource, String) {
    (
        std::cmp::Reverse(ordered_score::Key::new(hit.normalized_score)),
        hit.source,
        hit.source_id.clone(),
    )
}

/// A total order over the f32 score.
///
/// `f32` is only `PartialOrd`, and a `sort_by` with a `partial_cmp().unwrap_or(Equal)`
/// silently degrades to "leave them where they were" on a NaN — which is a
/// non-deterministic order, since the hits arrive in whatever order four threads
/// finished in. Mapping to a total key instead makes the sort deterministic even
/// if an adapter ever hands back a NaN.
mod ordered_score {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
    pub struct Key(u32);

    impl Key {
        /// Order-preserving map from f32 to u32. NaN sorts below every real
        /// score rather than anywhere.
        pub fn new(value: f32) -> Self {
            if value.is_nan() {
                return Key(0);
            }
            let bits = value.to_bits();
            // Flip the sign bit for positives, invert entirely for negatives:
            // the standard total-order encoding of IEEE-754 floats.
            let key = if bits & 0x8000_0000 == 0 {
                bits | 0x8000_0000
            } else {
                !bits
            };
            Key(key)
        }
    }
}

/// Fold duplicates together and find the disagreements among what is left.
pub fn merge(mut hits: Vec<RecallHit>) -> Merged {
    hits.sort_by_key(merge_order);

    let mut survivors: Vec<RecallHit> = Vec::new();
    let mut index_of_identity: BTreeMap<String, usize> = BTreeMap::new();

    for hit in hits {
        let identity = content_identity(&hit.content);
        match index_of_identity.get(&identity) {
            Some(&index) => fold_into(&mut survivors[index], hit),
            None => {
                index_of_identity.insert(identity, survivors.len());
                survivors.push(hit);
            }
        }
    }

    let mut conflicts = provenance_conflicts(&survivors);
    conflicts.extend(polarity_conflicts(&survivors));
    attach_conflict_indices(&mut survivors, &conflicts);

    Merged {
        hits: survivors,
        conflicts,
    }
}

/// Absorb a duplicate: keep the survivor's own fields, take the union of
/// provenance, and record where the copy came from.
///
/// Provenance is unioned rather than dropped because the duplicate is how we
/// learn that two stores' references name one artifact — discarding it would
/// throw away the only evidence linking, say, a memory id to a document id.
fn fold_into(survivor: &mut RecallHit, duplicate: RecallHit) {
    if survivor.source == duplicate.source && survivor.source_id == duplicate.source_id {
        return;
    }
    survivor.duplicates.push(HitRef {
        source: duplicate.source,
        source_id: duplicate.source_id.clone(),
    });
    survivor.duplicates.extend(duplicate.duplicates);
    survivor.duplicates.sort();
    survivor.duplicates.dedup();
    survivor.provenance_refs.extend(duplicate.provenance_refs);
    survivor.provenance_refs.sort();
    survivor.provenance_refs.dedup();
}

/// One provenance reference carrying more than one distinct content.
fn provenance_conflicts(hits: &[RecallHit]) -> Vec<RecallConflict> {
    let mut by_ref: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
    for (index, hit) in hits.iter().enumerate() {
        for reference in &hit.provenance_refs {
            by_ref.entry(reference.as_str()).or_default().push(index);
        }
    }

    by_ref
        .into_iter()
        .filter(|(_, members)| members.len() > 1)
        .filter_map(|(reference, members)| {
            let contents: Vec<String> = members
                .iter()
                .map(|&index| content_identity(&hits[index].content))
                .collect();
            let mut distinct = contents.clone();
            distinct.sort();
            distinct.dedup();
            if distinct.len() < 2 {
                return None;
            }
            Some(RecallConflict {
                kind: ConflictKind::DivergentContentForProvenance,
                identity: reference.to_string(),
                members: members.iter().map(|&index| member(&hits[index])).collect(),
                explanation: format!(
                    "{} hits cite {reference} with {} different contents",
                    members.len(),
                    distinct.len()
                ),
            })
        })
        .collect()
}

/// Same subject and predicate, asserted both ways.
///
/// Runs the crate's existing sentence parser over each hit's content so that a
/// contradiction found in recall is the same relation the knowledge graph would
/// have recorded, rather than a second opinion with its own grammar.
fn polarity_conflicts(hits: &[RecallHit]) -> Vec<RecallConflict> {
    let claims: Vec<Vec<ClaimRecord>> = hits.iter().map(claims_for).collect();
    let mut conflicts = Vec::new();

    for (left_index, left_claims) in claims.iter().enumerate() {
        for (right_offset, right_claims) in claims[left_index + 1..].iter().enumerate() {
            let right_index = left_index + 1 + right_offset;
            for left in left_claims {
                for right in right_claims {
                    if !contradicts(left, right) {
                        continue;
                    }
                    conflicts.push(RecallConflict {
                        kind: ConflictKind::OppositePolarity,
                        identity: format!(
                            "{} / {}",
                            left.normalized_subject, left.normalized_predicate
                        ),
                        members: vec![member(&hits[left_index]), member(&hits[right_index])],
                        explanation: format!(
                            "'{}' ({}) contradicts '{}' ({})",
                            left.text,
                            hits[left_index].source,
                            right.text,
                            hits[right_index].source
                        ),
                    });
                }
            }
        }
    }
    conflicts
}

/// Parse a hit's content as document text so the existing extractor can read it.
///
/// The synthetic chunk is never stored; it exists only to reuse
/// [`claim_extractor::extract_claims`] without duplicating its grammar here.
fn claims_for(hit: &RecallHit) -> Vec<ClaimRecord> {
    claim_extractor::extract_claims(&DocumentChunk {
        chunk_id: format!("{}:{}", hit.source, hit.source_id),
        document_id: hit.source.as_str().to_string(),
        content: hit.content.clone(),
        content_hash: content_identity(&hit.content),
    })
}

fn member(hit: &RecallHit) -> ConflictMember {
    ConflictMember {
        source: hit.source,
        source_id: hit.source_id.clone(),
        content: hit.content.clone(),
    }
}

/// Point each hit at the conflicts it appears in, so a caller reading one hit
/// cannot miss that it is disputed.
fn attach_conflict_indices(hits: &mut [RecallHit], conflicts: &[RecallConflict]) {
    for (conflict_index, conflict) in conflicts.iter().enumerate() {
        for member in &conflict.members {
            for hit in hits.iter_mut() {
                if hit.source == member.source && hit.source_id == member.source_id {
                    hit.conflicts.push(conflict_index);
                }
            }
        }
    }
    for hit in hits.iter_mut() {
        hit.conflicts.sort_unstable();
        hit.conflicts.dedup();
    }
}

#[cfg(test)]
#[path = "identity/tests.rs"]
mod tests;
