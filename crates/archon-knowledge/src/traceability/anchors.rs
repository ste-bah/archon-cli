//! Anchoring a requirement to `file:line`, and knowing when that anchor rotted.
//!
//! # Semantic search returns candidates, not proof
//!
//! A returned anchor is a citation to go verify. It is not a verdict, and its
//! relevance score is not evidence — nothing in this module reads
//! [`CodeHit::relevance_score`] to decide anything, and nothing downstream may
//! either. The score is carried only so a reader can see the ordering the index
//! produced. Letting it decide satisfaction is F1 with better maths.
//!
//! # Why anchoring goes through a port
//!
//! [`CodeSearch`] is a narrow read-only port, implemented in the command layer
//! over `archon_leann::search::Search::search_with_filter`. Two reasons, both
//! load-bearing:
//!
//! 1. This crate does not acquire an edge onto `archon-leann`, which drags in
//!    tree-sitter, tokio and an embedding provider.
//! 2. A test can anchor against fixture chunks with no embedder at all. Indexing
//!    is a genuine one-off cost — `Search::new` needs an `EmbeddingProvider`,
//!    and `archon-leann`'s file replacement holds the Cozo write lock across an
//!    entire `multi_transaction` — so no test in this crate may index anything,
//!    and with a port none can.
//!
//! # `file_hash` is what makes an edge invalidatable
//!
//! Every anchor records the SHA-256 of its whole file, the same quantity
//! `archon-leann` records in `CodeMetadata::file_hash`. When the file changes,
//! [`check_freshness`] reports the edge as stale instead of letting it keep
//! naming a line number that has since moved. A stale edge collapses to
//! `Unproven`: known-stale beats silently-wrong, and neither counts.

use std::path::Path;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::errors::Result;
use crate::schema::RelationRecord;
use crate::{now_iso, stable_id};

use super::requirements::Requirement;
use super::tasks::TaskBinding;

/// `relation_type` for a requirement→code edge in `kb_relations`.
pub const ANCHOR_RELATION_TYPE: &str = "anchored_in";

/// One hit from the code index.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CodeHit {
    pub file_path: String,
    pub language: String,
    pub line_start: usize,
    pub line_end: usize,
    /// Carried for display only. Never read by any promotion decision.
    pub relevance_score: f64,
}

/// A read-only view of an already-built code index.
///
/// Deliberately has no `index` method. Indexing runs out of band; a report that
/// could index would take the longest write lock in the repository while a
/// workflow was running.
pub trait CodeSearch {
    /// `path_pattern` is a substring filter over the indexed path, matching
    /// `archon-leann`'s own semantics — not a glob.
    fn search(&self, query: &str, limit: usize, path_pattern: Option<&str>)
    -> Result<Vec<CodeHit>>;
}

/// A requirement→code edge: a citation with a hash that can invalidate it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Anchor {
    pub requirement_id: String,
    /// The task whose declared paths scoped the search that found this.
    pub task_id: String,
    /// Repository-relative, forward slashes.
    pub file_path: String,
    pub line_start: usize,
    pub line_end: usize,
    /// SHA-256 of the whole file at anchor time.
    pub file_hash: String,
    /// The declared path scope this hit fell inside.
    pub path_scope: String,
    pub relevance_score: f64,
}

impl Anchor {
    /// `path:start-end`, the identity a reviewer checks and the key duplicate
    /// detection groups on.
    pub fn citation(&self) -> String {
        format!("{}:{}-{}", self.file_path, self.line_start, self.line_end)
    }
}

/// Whether an anchor still describes the file it was taken from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AnchorFreshness {
    /// The file hashes to what was recorded.
    Fresh,
    /// The file changed. The edge names a line that may have moved.
    Stale { recorded: String, current: String },
    /// The file is gone or unreadable.
    FileMissing,
}

impl AnchorFreshness {
    pub fn is_fresh(&self) -> bool {
        matches!(self, AnchorFreshness::Fresh)
    }
}

/// SHA-256 of file bytes, hex-encoded — the same quantity `archon-leann`
/// computes for `CodeMetadata::file_hash`, so the two agree on staleness.
pub fn file_hash(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}

/// Re-hash an anchor's file and compare.
pub fn check_freshness(anchor: &Anchor, repo_root: &Path) -> AnchorFreshness {
    match std::fs::read(repo_root.join(&anchor.file_path)) {
        Err(_) => AnchorFreshness::FileMissing,
        Ok(bytes) => {
            let current = file_hash(&bytes);
            if current == anchor.file_hash {
                AnchorFreshness::Fresh
            } else {
                AnchorFreshness::Stale {
                    recorded: anchor.file_hash.clone(),
                    current,
                }
            }
        }
    }
}

/// Why a requirement produced no anchors.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AnchorGap {
    /// The implementing task declares no paths, so there is nothing to scope a
    /// search to. Searching the whole repository instead would return the
    /// generic top hit for every requirement — which is the shape of F1.
    NoDeclaredPaths { task_id: String },
    /// The index returned nothing inside the declared paths.
    NoHitInScope { task_id: String },
    /// No task claims this requirement at all.
    Unclaimed,
    /// No code index was consulted, so no anchor could exist.
    ///
    /// Distinct from [`AnchorGap::NoHitInScope`], and the distinction matters:
    /// "we did not look" and "we looked and found nothing" are different facts,
    /// and reporting the first as the second would understate the code without
    /// evidence.
    IndexNotConsulted,
}

/// Anchor one requirement inside one task's declared paths.
///
/// One query per declared path scope, capped by `max_scopes`. A hit is kept only
/// when its path actually falls inside a declared scope: `path_pattern` is a
/// substring filter on the index side, so the containment check is repeated here
/// rather than trusted. A hit whose file cannot be read is dropped — an anchor
/// that cannot be hashed cannot be invalidated, and an edge that can never go
/// stale is exactly the silently-wrong edge this design refuses to create.
pub fn anchor_requirement(
    search: &dyn CodeSearch,
    requirement: &Requirement,
    binding: &TaskBinding,
    repo_root: &Path,
    limit_per_scope: usize,
    max_scopes: usize,
) -> Result<std::result::Result<Vec<Anchor>, AnchorGap>> {
    if binding.path_scopes.is_empty() {
        return Ok(Err(AnchorGap::NoDeclaredPaths {
            task_id: binding.task_id.clone(),
        }));
    }

    let query = format!("{} {}", requirement.id, requirement.text);
    let mut anchors: Vec<Anchor> = Vec::new();

    for scope in binding.path_scopes.iter().take(max_scopes) {
        for hit in search.search(&query, limit_per_scope, Some(scope))? {
            let path = hit.file_path.replace('\\', "/");
            if !path.contains(scope.as_str()) {
                continue;
            }
            let Ok(bytes) = std::fs::read(repo_root.join(&path)) else {
                continue;
            };
            let anchor = Anchor {
                requirement_id: requirement.id.clone(),
                task_id: binding.task_id.clone(),
                file_path: path,
                line_start: hit.line_start,
                line_end: hit.line_end,
                file_hash: file_hash(&bytes),
                path_scope: scope.clone(),
                relevance_score: hit.relevance_score,
            };
            if !anchors.iter().any(|a| a.citation() == anchor.citation()) {
                anchors.push(anchor);
            }
        }
    }

    if anchors.is_empty() {
        return Ok(Err(AnchorGap::NoHitInScope {
            task_id: binding.task_id.clone(),
        }));
    }
    Ok(Ok(anchors))
}

/// Project an anchor into the knowledge graph as a relation.
///
/// `source_chunk_id` is the `file:line` citation, so the edge names its evidence
/// in the record itself. `confidence` is fixed at 1.0 and means "this edge was
/// recorded", not "this requirement is satisfied" — satisfaction is the proof
/// level, which is a separate, ordered, non-numeric fact.
pub fn anchor_relation(anchor: &Anchor, requirement_entity_id: &str) -> RelationRecord {
    RelationRecord {
        relation_id: anchor_relation_id(anchor),
        source_entity_id: requirement_entity_id.to_string(),
        target_entity_id: anchor.citation(),
        relation_type: ANCHOR_RELATION_TYPE.to_string(),
        source_chunk_id: anchor.citation(),
        confidence: 1.0,
        created_at: now_iso(),
    }
}

/// Stable identity for an anchor edge: same requirement, same citation, same id.
pub fn anchor_relation_id(anchor: &Anchor) -> String {
    stable_id(
        "reqanchor",
        &[
            anchor.requirement_id.as_str(),
            anchor.task_id.as_str(),
            &anchor.citation(),
        ],
    )
}

#[cfg(test)]
mod tests;
