//! Atomic persistence for filed Q&A answer nodes and their content-hash owner.

use std::collections::BTreeMap;

use anyhow::Result;
use cozo::{DataValue, DbInstance, MultiTransaction};

pub(super) fn reserve_answer_node(
    db: &DbInstance,
    node_id: &str,
    title: &str,
    content: &str,
    content_hash: &str,
    now: f64,
) -> Result<String> {
    let transaction = db.multi_transaction(true);
    match reserve_answer_node_in_transaction(
        &transaction,
        node_id,
        title,
        content,
        content_hash,
        now,
    ) {
        Ok(owner_id) => transaction
            .commit()
            .map(|()| owner_id)
            .map_err(|error| anyhow::anyhow!("commit answer filing failed: {error}")),
        Err(error) => {
            let _ = transaction.abort();
            Err(error)
        }
    }
}

fn reserve_answer_node_in_transaction(
    transaction: &MultiTransaction,
    node_id: &str,
    title: &str,
    content: &str,
    content_hash: &str,
    now: f64,
) -> Result<String> {
    let mut hash_params = BTreeMap::new();
    hash_params.insert("content_hash".into(), DataValue::from(content_hash));
    let existing = transaction
        .run_script(
            "?[node_id] := *kb_content_hashes{content_hash, node_id}, content_hash = $content_hash",
            hash_params,
        )
        .map_err(|error| anyhow::anyhow!("read answer hash owner failed: {error}"))?;
    if let Some(owner_id) = existing.rows.first().and_then(|row| row[0].get_str()) {
        return Ok(owner_id.to_string());
    }

    let mut params = BTreeMap::new();
    params.insert("node_id".into(), DataValue::from(node_id));
    params.insert("node_type".into(), DataValue::from("answer"));
    params.insert("source".into(), DataValue::from("qa-engine"));
    params.insert("domain_tag".into(), DataValue::from(""));
    params.insert("title".into(), DataValue::from(title));
    params.insert("content".into(), DataValue::from(content));
    params.insert("content_hash".into(), DataValue::from(content_hash));
    params.insert("chunk_index".into(), DataValue::from(0i64));
    params.insert("created_at".into(), DataValue::from(now));
    params.insert("updated_at".into(), DataValue::from(now));
    transaction
        .run_script(
            "?[content_hash, node_id] <- [[$content_hash, $node_id]] \
             :insert kb_content_hashes { content_hash => node_id }",
            params.clone(),
        )
        .map_err(|error| anyhow::anyhow!("reserve answer content hash failed: {error}"))?;
    transaction
        .run_script(
            "?[node_id, node_type, source, domain_tag, title, content, \
             content_hash, chunk_index, created_at, updated_at] <- \
             [[$node_id, $node_type, $source, $domain_tag, $title, $content, \
             $content_hash, $chunk_index, $created_at, $updated_at]] \
             :put kb_nodes { node_id => node_type, source, domain_tag, title, \
             content, content_hash, chunk_index, created_at, updated_at }",
            params,
        )
        .map_err(|error| anyhow::anyhow!("insert answer node failed: {error}"))?;
    Ok(node_id.to_string())
}
