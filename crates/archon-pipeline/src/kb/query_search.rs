use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use anyhow::Result;
use archon_docs::embed::LocalEmbeddingProvider;
use cozo::{DataValue, DbInstance, ScriptMutability, Vector};
use ndarray::Array1;

use super::query::{ScoredKbNode, row_to_kb_node};
use super::schema::KbNodeType;

pub(super) fn search_nodes(
    db: &DbInstance,
    embedder: Option<&Arc<dyn LocalEmbeddingProvider>>,
    query_text: &str,
    limit: usize,
    type_filter: Option<&[KbNodeType]>,
) -> Result<Vec<ScoredKbNode>> {
    if limit == 0 {
        return Ok(Vec::new());
    }
    let mut candidates = text_candidates(db, query_text, limit, type_filter)?;
    if let Some(embedder) = embedder {
        let _guard = super::schema::lock_embedding_state()?;
        super::schema::assert_embedding_space(
            db,
            &embedder.embedding_space_id(),
            embedder.dimension(),
        )?;
        super::ingest::backfill_missing_embeddings(db, embedder.as_ref())?;
        merge_semantic_candidates(
            db,
            embedder.as_ref(),
            query_text,
            limit,
            type_filter,
            &mut candidates,
        )?;
    }
    candidates.retain(|candidate| {
        type_filter.is_none_or(|filter| filter.contains(&candidate.node.node_type))
            && candidate.score > 0.0
    });
    for candidate in &mut candidates {
        if candidate.node.node_type == KbNodeType::Answer {
            candidate.score *= 0.9;
        }
    }
    candidates.sort_by(|a, b| {
        b.score
            .total_cmp(&a.score)
            .then_with(|| a.node.node_id.cmp(&b.node.node_id))
    });
    candidates.truncate(limit);
    Ok(candidates)
}

#[cfg(test)]
pub(super) fn text_candidates_for_tests(
    db: &DbInstance,
    query_text: &str,
    limit: usize,
) -> Result<Vec<ScoredKbNode>> {
    text_candidates(db, query_text, limit, None)
}

fn text_candidates(
    db: &DbInstance,
    query_text: &str,
    limit: usize,
    type_filter: Option<&[KbNodeType]>,
) -> Result<Vec<ScoredKbNode>> {
    let mut params = BTreeMap::new();
    params.insert("q".to_string(), DataValue::from(query_text.to_lowercase()));
    params.insert("lim".to_string(), DataValue::from(candidate_limit(limit)?));
    let type_names = type_filter.map(|filter| {
        DataValue::List(
            filter
                .iter()
                .map(|node_type| DataValue::from(node_type_name(node_type)))
                .collect(),
        )
    });
    let type_clause = if let Some(type_names) = type_names {
        params.insert("types".to_string(), type_names);
        ", node_type in $types"
    } else {
        ""
    };
    let script = format!(
        "?[node_id, node_type, source, domain_tag, title, content, \
         content_hash, chunk_index, created_at, updated_at, title_match, content_match] := \
         *kb_nodes{{node_id, node_type, source, domain_tag, title, content, \
         content_hash, chunk_index, created_at, updated_at}}, \
         title_match = str_includes(lowercase(title), $q), \
         content_match = str_includes(lowercase(content), $q), \
         (title_match or content_match){type_clause} \
         :order -title_match, -content_match, node_id \
         :limit $lim"
    );
    let result = db
        .run_script(&script, params, ScriptMutability::Immutable)
        .map_err(|error| anyhow::anyhow!("KB text search failed: {error}"))?;
    Ok(score_text_rows(&result.rows, query_text))
}

fn score_text_rows(rows: &[Vec<DataValue>], query_text: &str) -> Vec<ScoredKbNode> {
    let mut candidates: Vec<_> = rows
        .iter()
        .map(|row| {
            let node = row_to_kb_node(row);
            let score = text_score(&node, query_text);
            ScoredKbNode { node, score }
        })
        .collect();
    candidates.sort_by(|a, b| {
        b.score
            .total_cmp(&a.score)
            .then_with(|| a.node.node_id.cmp(&b.node.node_id))
    });
    candidates
}

fn text_score(node: &super::schema::KbNode, query_text: &str) -> f64 {
    let query = query_text.to_lowercase();
    let title = node.title.to_lowercase().contains(&query) as u8;
    let content = node.content.to_lowercase().contains(&query) as u8;
    (f64::from(title) * 0.8 + f64::from(content) * 0.5).min(1.0)
}

fn merge_semantic_candidates(
    db: &DbInstance,
    embedder: &dyn LocalEmbeddingProvider,
    query_text: &str,
    limit: usize,
    type_filter: Option<&[KbNodeType]>,
    candidates: &mut Vec<ScoredKbNode>,
) -> Result<()> {
    let query = embedder.embed_query(query_text)?;
    let total = embedding_count(db)?;
    let k = candidate_limit_usize(limit)?.min(total);
    if k > 0 {
        let result = semantic_candidates(db, &query, k, type_filter)?;
        merge_semantic_rows(&result.rows, type_filter, candidates);
    }
    Ok(())
}

fn embedding_count(db: &DbInstance) -> Result<usize> {
    let result = db
        .run_script(
            "?[count(node_id)] := *kb_embeddings{node_id}",
            Default::default(),
            ScriptMutability::Immutable,
        )
        .map_err(|error| anyhow::anyhow!("count KB embeddings failed: {error}"))?;
    Ok(result
        .rows
        .first()
        .and_then(|row| row[0].get_int())
        .unwrap_or(0) as usize)
}

fn semantic_candidates(
    db: &DbInstance,
    query: &[f32],
    k: usize,
    type_filter: Option<&[KbNodeType]>,
) -> Result<cozo::NamedRows> {
    let mut params = BTreeMap::new();
    params.insert(
        "query".to_string(),
        DataValue::Vec(Vector::F32(Array1::from_vec(query.to_vec()))),
    );
    params.insert("k".to_string(), DataValue::from(i64::try_from(k)?));
    let type_names = type_filter.map(|filter| {
        DataValue::List(
            filter
                .iter()
                .map(|node_type| DataValue::from(node_type_name(node_type)))
                .collect(),
        )
    });
    let type_clause = if let Some(type_names) = type_names {
        params.insert("types".to_string(), type_names);
        ", node_type in $types"
    } else {
        ""
    };
    let script = if type_filter.is_some() {
        format!(
            "?[node_id, node_type, source, domain_tag, title, content, \
             content_hash, chunk_index, created_at, updated_at, distance] := \
             *kb_embeddings{{node_id, embedding}}, \
             *kb_nodes{{node_id, node_type, source, domain_tag, title, content, \
             content_hash, chunk_index, created_at, updated_at}}{type_clause}, \
             distance = cos_dist(embedding, $query) \
             :order distance, node_id \
             :limit $k"
        )
    } else {
        "?[node_id, node_type, source, domain_tag, title, content, \
         content_hash, chunk_index, created_at, updated_at, distance] := \
         ~kb_embeddings:semantic_idx{node_id | query: $query, k: $k, ef: 50, \
         bind_distance: distance}, \
         *kb_nodes{node_id, node_type, source, domain_tag, title, content, \
         content_hash, chunk_index, created_at, updated_at}"
            .to_string()
    };
    db.run_script(&script, params, ScriptMutability::Immutable)
        .map_err(|error| anyhow::anyhow!("KB semantic search failed: {error}"))
}

pub(super) fn merge_semantic_rows(
    rows: &[Vec<DataValue>],
    type_filter: Option<&[KbNodeType]>,
    candidates: &mut Vec<ScoredKbNode>,
) {
    let mut by_id: HashMap<String, usize> = candidates
        .iter()
        .enumerate()
        .map(|(index, candidate)| (candidate.node.node_id.clone(), index))
        .collect();
    for row in rows {
        let node = row_to_kb_node(row);
        if type_filter.is_some_and(|filter| !filter.contains(&node.node_type)) {
            continue;
        }
        let Some(score) = semantic_score(row[10].get_float().unwrap_or(f64::INFINITY)) else {
            continue;
        };
        if let Some(index) = by_id.get(&node.node_id).copied() {
            candidates[index].score = candidates[index].score.max(score);
        } else {
            by_id.insert(node.node_id.clone(), candidates.len());
            candidates.push(ScoredKbNode { node, score });
        }
    }
}

fn semantic_score(distance: f64) -> Option<f64> {
    if !distance.is_finite() || !(0.0..=2.0).contains(&distance) {
        return None;
    }
    Some(1.0 - distance / 2.0).filter(|score| *score > 0.0)
}

#[cfg(test)]
pub(super) fn candidate_limit_for_tests(limit: usize) -> Result<usize> {
    candidate_limit_usize(limit)
}

fn candidate_limit_usize(limit: usize) -> Result<usize> {
    Ok(usize::try_from(candidate_limit(limit)?)?)
}

fn candidate_limit(limit: usize) -> Result<i64> {
    let expanded = limit
        .checked_mul(3)
        .ok_or_else(|| anyhow::anyhow!("KB search limit is too large"))?;
    i64::try_from(expanded).map_err(|_| anyhow::anyhow!("KB search limit is too large"))
}

fn node_type_name(node_type: &KbNodeType) -> &'static str {
    match node_type {
        KbNodeType::Raw => "raw",
        KbNodeType::Compiled => "compiled",
        KbNodeType::Concept => "concept",
        KbNodeType::Answer => "answer",
        KbNodeType::Index => "index",
    }
}
