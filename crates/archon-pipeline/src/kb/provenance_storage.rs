//! Batched persistence for filed-answer provenance edges.

use std::collections::{BTreeMap, HashSet};

use anyhow::Result;
use cozo::{DataValue, DbInstance, MultiTransaction};

pub(super) fn persist_derived_from_edges(
    db: &DbInstance,
    owner_id: &str,
    source_node_ids: &[String],
    now: f64,
) -> Result<()> {
    let source_node_ids = unique_source_ids(source_node_ids);
    if source_node_ids.is_empty() {
        return Ok(());
    }

    let transaction = db.multi_transaction(true);
    match persist_derived_from_edges_in_transaction(&transaction, owner_id, &source_node_ids, now) {
        Ok(()) => match transaction.commit() {
            Ok(()) => Ok(()),
            Err(error) => {
                let _ = transaction.abort();
                Err(anyhow::anyhow!(
                    "commit DerivedFrom edge batch failed: {error}"
                ))
            }
        },
        Err(error) => {
            let _ = transaction.abort();
            Err(error)
        }
    }
}

fn persist_derived_from_edges_in_transaction(
    transaction: &MultiTransaction,
    owner_id: &str,
    source_node_ids: &[&str],
    now: f64,
) -> Result<()> {
    let existing_targets = existing_targets(transaction, owner_id)?;
    let rows = edge_rows(owner_id, source_node_ids, &existing_targets, now);
    if rows.is_empty() {
        return Ok(());
    }

    record_write_attempt();
    let mut params = BTreeMap::new();
    params.insert("rows".into(), DataValue::List(rows));
    transaction
        .run_script(
            "?[edge_id, source_node_id, target_node_id, edge_type, created_at] <- $rows \
             :put kb_edges { edge_id => source_node_id, target_node_id, edge_type, created_at }",
            params,
        )
        .map_err(|error| anyhow::anyhow!("batch DerivedFrom edge persistence failed: {error}"))?;
    record_post_batch_write();
    if should_fail_after_batch_write() {
        anyhow::bail!("injected provenance failure after batch write");
    }
    Ok(())
}

fn unique_source_ids(source_node_ids: &[String]) -> Vec<&str> {
    let mut seen = HashSet::new();
    source_node_ids
        .iter()
        .filter_map(|source_id| {
            seen.insert(source_id.as_str())
                .then_some(source_id.as_str())
        })
        .collect()
}

fn existing_targets(transaction: &MultiTransaction, owner_id: &str) -> Result<HashSet<String>> {
    let mut params = BTreeMap::new();
    params.insert("owner_id".into(), DataValue::from(owner_id));
    let result = transaction
        .run_script(
            "?[target_node_id] := *kb_edges{source_node_id, target_node_id, edge_type}, \
             source_node_id = $owner_id, edge_type = 'DerivedFrom'",
            params,
        )
        .map_err(|error| anyhow::anyhow!("read existing DerivedFrom edges failed: {error}"))?;
    Ok(result
        .rows
        .iter()
        .filter_map(|row| row[0].get_str().map(str::to_string))
        .collect())
}

fn edge_rows(
    owner_id: &str,
    source_node_ids: &[&str],
    existing_targets: &HashSet<String>,
    now: f64,
) -> Vec<DataValue> {
    source_node_ids
        .iter()
        .filter(|source_id| !existing_targets.contains(**source_id))
        .map(|source_id| {
            DataValue::List(vec![
                DataValue::from(derived_from_edge_id(owner_id, source_id)),
                DataValue::from(owner_id),
                DataValue::from(*source_id),
                DataValue::from("DerivedFrom"),
                DataValue::from(now),
            ])
        })
        .collect()
}

fn derived_from_edge_id(owner_id: &str, target_id: &str) -> String {
    use sha2::{Digest, Sha256};

    let mut hasher = Sha256::new();
    hasher.update(owner_id.as_bytes());
    hasher.update([0]);
    hasher.update(target_id.as_bytes());
    hasher.update([0]);
    hasher.update(b"DerivedFrom");
    format!("edge-{}", hex::encode(hasher.finalize()))
}
#[cfg(test)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct BatchCounts {
    pub write_queries: usize,
    pub post_batch_writes: usize,
}

#[cfg(test)]
mod test_state {
    use std::cell::RefCell;

    use super::BatchCounts;

    #[derive(Default)]
    pub(super) struct State {
        pub(super) counts: BatchCounts,
        pub(super) fail_after_batch_write: bool,
    }

    std::thread_local! {
        pub(super) static STATE: RefCell<State> = RefCell::new(State::default());
    }
}

fn record_write_attempt() {
    #[cfg(test)]
    test_state::STATE.with(|state| state.borrow_mut().counts.write_queries += 1);
}

fn record_post_batch_write() {
    #[cfg(test)]
    test_state::STATE.with(|state| state.borrow_mut().counts.post_batch_writes += 1);
}

fn should_fail_after_batch_write() -> bool {
    #[cfg(test)]
    {
        test_state::STATE
            .with(|state| std::mem::take(&mut state.borrow_mut().fail_after_batch_write))
    }
    #[cfg(not(test))]
    false
}

#[cfg(test)]
pub(super) mod test_support {
    use super::{BatchCounts, test_state};

    pub(crate) fn reset() {
        test_state::STATE.with(|state| *state.borrow_mut() = Default::default());
    }

    pub(crate) fn counts() -> BatchCounts {
        test_state::STATE.with(|state| state.borrow().counts)
    }

    pub(crate) fn fail_after_batch_write() {
        test_state::STATE.with(|state| state.borrow_mut().fail_after_batch_write = true);
    }
}
