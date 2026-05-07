use std::collections::BTreeMap;

use cozo::{DataValue, DbInstance, ScriptMutability};
use tracing::{info, warn};

use crate::centroid_builder;
use crate::errors::{ConstellationError, Result};
use crate::store::{self, ConstellationCentroid};

pub const MIN_BOOTSTRAP_TEXTS: usize = 3;

#[derive(Clone, Copy)]
pub enum BootstrapSource<'a> {
    /// Pre-collected representative texts. Used by explicit CLI bootstrap.
    Inline(&'a [String]),
    /// Last `limit` memory texts. target = "memory" maps here.
    RecentMemories { db: &'a DbInstance, limit: usize },
    /// Last `limit` doc chunks. target = "docs" maps here.
    RecentDocChunks { db: &'a DbInstance, limit: usize },
    /// Active session transcript. target = "session" requires an explicit session id.
    SessionTranscript {
        db: &'a DbInstance,
        session_id: &'a str,
        limit: usize,
    },
}

impl BootstrapSource<'_> {
    fn source_relation(self) -> &'static str {
        match self {
            Self::Inline(_) => "inline",
            Self::RecentMemories { .. } => "memories",
            Self::RecentDocChunks { .. } => "doc_chunks",
            Self::SessionTranscript { .. } => "messages",
        }
    }
}

pub fn bootstrap_centroid(
    db: &DbInstance,
    target: &str,
    source: BootstrapSource<'_>,
) -> Result<Option<ConstellationCentroid>> {
    validate_bootstrap_target(target)?;
    crate::ensure_schema(db)?;

    let source_relation = source.source_relation();
    let texts = collect_texts(source)?;
    if texts.len() < MIN_BOOTSTRAP_TEXTS {
        warn!(
            target,
            source_relation,
            text_count = texts.len(),
            min_texts = MIN_BOOTSTRAP_TEXTS,
            "constellation.bootstrap.skip"
        );
        return Ok(None);
    }

    let vector = centroid_builder::centroid_vector(&texts)
        .ok_or_else(|| ConstellationError::Store("no bootstrap vectors produced".into()))?;
    let version = store::next_version(db, target)?;
    let sample_ids = texts
        .iter()
        .enumerate()
        .map(|(idx, text)| {
            crate::stable_id(
                "bootstrap-sample",
                &[target, source_relation, &idx.to_string(), text],
            )
        })
        .collect::<Vec<_>>();
    let centroid_id = crate::stable_id(
        "constellation",
        &[
            target,
            source_relation,
            &version.to_string(),
            &sample_ids.join("|"),
        ],
    );
    let centroid = ConstellationCentroid {
        centroid_id,
        target: target.to_string(),
        version,
        vector,
        sample_ids,
        sample_count: texts.len(),
        source_relation: source_relation.to_string(),
        created_at: chrono::Utc::now().to_rfc3339(),
    };
    store::insert_centroid(db, &centroid)?;
    store::insert_vector(db, &centroid.centroid_id, &centroid.vector)?;
    info!(
        target,
        source_relation,
        centroid_id = %centroid.centroid_id,
        version = centroid.version,
        sample_count = centroid.sample_count,
        "constellation.bootstrap.ok"
    );
    Ok(Some(centroid))
}

pub(crate) fn default_bootstrap_source<'a>(
    db: &'a DbInstance,
    target: &str,
) -> Result<BootstrapSource<'a>> {
    match target {
        "memory" => Ok(BootstrapSource::RecentMemories { db, limit: 50 }),
        "docs" => Ok(BootstrapSource::RecentDocChunks { db, limit: 50 }),
        "session" => Err(ConstellationError::NeedsExplicitSession(target.to_string())),
        "project" | "research-domain" | "strategic-workflow" => {
            Err(ConstellationError::MissingCentroid(target.to_string()))
        }
        other => Err(ConstellationError::UnknownTarget(other.to_string())),
    }
}

fn validate_bootstrap_target(target: &str) -> Result<()> {
    if crate::is_known_target(target) {
        Ok(())
    } else {
        Err(ConstellationError::UnknownTarget(target.to_string()))
    }
}

fn collect_texts(source: BootstrapSource<'_>) -> Result<Vec<String>> {
    match source {
        BootstrapSource::Inline(texts) => Ok(clean_texts(texts.iter().cloned())),
        BootstrapSource::RecentMemories { db, limit } => {
            query_recent_texts(db, "memories", "content", Some("created_at"), None, limit)
        }
        BootstrapSource::RecentDocChunks { db, limit } => query_recent_texts(
            db,
            "doc_chunks",
            "content",
            Some("chunk_index"),
            None,
            limit,
        ),
        BootstrapSource::SessionTranscript {
            db,
            session_id,
            limit,
        } => query_recent_texts(
            db,
            "messages",
            "content",
            Some("message_index"),
            Some(("session_id", session_id)),
            limit,
        ),
    }
}

fn query_recent_texts(
    db: &DbInstance,
    relation: &str,
    text_field: &str,
    sort_field: Option<&str>,
    filter: Option<(&str, &str)>,
    limit: usize,
) -> Result<Vec<String>> {
    let limit = limit.max(1);
    let mut params = BTreeMap::new();
    let filter_clause = if let Some((field, value)) = filter {
        params.insert("filter_value".to_string(), DataValue::from(value));
        format!(", {field} = $filter_value")
    } else {
        String::new()
    };
    let sort_clause = sort_field
        .map(|field| format!(":sort -{field}"))
        .unwrap_or_default();
    let query = format!(
        "?[text] := *{relation}{{{text_field}{filter_fields}}}{filter_clause} {sort_clause} :limit {limit}",
        filter_fields = filter
            .map(|(field, _)| format!(", {field}"))
            .unwrap_or_default()
    );

    let result = db.run_script(&query, params, ScriptMutability::Immutable);
    match result {
        Ok(rows) => Ok(clean_texts(rows.rows.iter().filter_map(|row| {
            row.first()
                .and_then(DataValue::get_str)
                .map(ToString::to_string)
        }))),
        Err(err) if relation_is_missing(&err.to_string(), relation) => {
            warn!(
                relation,
                error = %err,
                "constellation.bootstrap.skip source relation unavailable"
            );
            Ok(Vec::new())
        }
        Err(err) => Err(ConstellationError::Store(format!(
            "bootstrap query failed for {relation}: {err}"
        ))),
    }
}

fn clean_texts<I>(texts: I) -> Vec<String>
where
    I: IntoIterator<Item = String>,
{
    texts
        .into_iter()
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty())
        .collect()
}

fn relation_is_missing(message: &str, relation: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    lower.contains(&relation.to_ascii_lowercase())
        && (lower.contains("not found")
            || lower.contains("does not exist")
            || lower.contains("undefined")
            || lower.contains("unable to find"))
}

#[cfg(test)]
mod tests {
    use cozo::DbInstance;
    use tracing_test::traced_test;

    use super::*;

    fn db() -> DbInstance {
        DbInstance::new("mem", "", Default::default()).unwrap()
    }

    fn texts(count: usize) -> Vec<String> {
        (0..count)
            .map(|idx| format!("representative memory signal {idx} with stable context"))
            .collect()
    }

    #[test]
    fn bootstrap_centroid_writes_centroid_to_store() {
        let db = db();
        let texts = texts(5);

        let centroid = bootstrap_centroid(&db, "memory", BootstrapSource::Inline(&texts)).unwrap();

        assert!(centroid.is_some());
        assert_eq!(store::list_centroids(&db).unwrap().len(), 1);
        assert_eq!(store::count_vectors(&db).unwrap(), 1);
    }

    #[traced_test]
    #[test]
    fn bootstrap_centroid_skips_when_texts_below_min() {
        let db = db();
        let texts = texts(2);

        let centroid = bootstrap_centroid(&db, "memory", BootstrapSource::Inline(&texts)).unwrap();

        assert!(centroid.is_none());
        assert!(store::list_centroids(&db).unwrap().is_empty());
        assert!(logs_contain("constellation.bootstrap.skip"));
    }

    #[test]
    fn bootstrap_centroid_unknown_target_errors() {
        let db = db();
        let texts = texts(5);

        let err = bootstrap_centroid(&db, "wat", BootstrapSource::Inline(&texts)).unwrap_err();

        assert!(matches!(err, ConstellationError::UnknownTarget(target) if target == "wat"));
    }
}
