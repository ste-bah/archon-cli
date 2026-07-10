use std::collections::BTreeMap;
use std::path::Path;

use crate::write_coordinator::worktree_isolation::CanonicalBaseline;
use crate::write_coordinator::write_plan::NormalizedPath;

type HashPair = (BTreeMap<String, String>, BTreeMap<String, String>);

pub(super) fn target_hashes(
    isolated: &Path,
    declared_targets: &[NormalizedPath],
    baseline: &CanonicalBaseline,
) -> HashPair {
    let mut pre = BTreeMap::new();
    let mut post = BTreeMap::new();
    for target in declared_targets {
        let rel = target.as_str();
        let post_hash = hash_existing_file(&isolated.join(&rel));
        let pre_hash = baseline
            .declared_target_meta
            .get(&rel)
            .and_then(|meta| non_empty_hash(&meta.blake3_hex));
        push_changed_hash(&mut pre, &mut post, rel, pre_hash, post_hash);
    }
    (pre, post)
}

fn non_empty_hash(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn push_changed_hash(
    pre: &mut BTreeMap<String, String>,
    post: &mut BTreeMap<String, String>,
    rel: String,
    pre_hash: Option<String>,
    post_hash: Option<String>,
) {
    if pre_hash.is_none() && post_hash.is_none() {
        return;
    }
    pre.insert(
        rel.clone(),
        pre_hash.unwrap_or_else(|| "absent".to_string()),
    );
    post.insert(rel, post_hash.unwrap_or_else(|| "deleted".to_string()));
}

fn hash_existing_file(path: &Path) -> Option<String> {
    if !path.is_file() {
        return None;
    }
    std::fs::read(path)
        .ok()
        .map(|bytes| blake3::hash(&bytes).to_hex().to_string())
}
