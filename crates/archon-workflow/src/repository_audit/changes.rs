//! Content changes between sealed views; never read the live checkout.
use super::runtime::Snapshot;
use crate::{WorkflowError, WorkflowResult};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

fn index(snapshot: &Snapshot) -> WorkflowResult<BTreeMap<String, String>> {
    if snapshot.root.join(".git").exists() {
        return snapshot.content_index();
    }
    // Explicit non-repository snapshots have no Git object database.
    snapshot
        .paths
        .iter()
        .map(|path| {
            let bytes = std::fs::read(snapshot.root.join(path))
                .map_err(|error| WorkflowError::io(snapshot.root.join(path), error))?;
            Ok((path.clone(), blake3::hash(&bytes).to_hex().to_string()))
        })
        .collect()
}
pub(super) fn between(
    previous: Option<&Snapshot>,
    current: &Snapshot,
) -> WorkflowResult<Vec<Value>> {
    let Some(previous) = previous else {
        return Ok(Vec::new());
    };
    if previous.identity == current.identity {
        return Ok(Vec::new());
    }
    let old = index(previous)?;
    let new = index(current)?;
    Ok(old
        .keys()
        .chain(new.keys())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .filter(|path| old.get(*path) != new.get(*path))
        .map(|path| {
            json!({"path":path,"kind":match (old.contains_key(path),new.contains_key(path)) {
                (false,true)=>"created",(true,false)=>"deleted",_=>"modified",
            }})
        })
        .collect())
}
