//! Game-theory specimen library ingest and query.
//!
//! The source of truth is the "Specimen Library (known fingerprints)" table in
//! `.archon/agents/gametheory/game-classifier.md`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::Result;
use cozo::{DataValue, DbInstance, ScriptMutability};
use serde::{Deserialize, Serialize};

use super::schema::ensure_gametheory_schema;

const SPECIMEN_SOURCE_RELATIVE: &str = ".archon/agents/gametheory/game-classifier.md";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpecimenRecord {
    pub specimen_id: String,
    pub situation_type: String,
    pub cooperation: String,
    pub payoff_sum: String,
    pub symmetry: String,
    pub timing: String,
    pub perfect_info: String,
    pub complete_info: String,
    pub cardinality: String,
    pub strategy_space: String,
    pub horizon: String,
    pub primary_family: String,
    pub notes: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpecimenLoadResult {
    pub inserted: usize,
    pub total: usize,
}

/// Ensure the specimen table is populated, lazily loading from the canonical
/// markdown source only when the table is empty or `force` is true.
pub fn ensure_specimen_library_loaded(
    db: &DbInstance,
    force: bool,
) -> Result<SpecimenLoadResult> {
    ensure_gametheory_schema(db)?;
    let existing = count_specimens(db)?;
    if existing > 0 && !force {
        return Ok(SpecimenLoadResult {
            inserted: 0,
            total: existing,
        });
    }

    let source = resolve_specimen_source_path()?;
    let markdown = std::fs::read_to_string(&source)
        .map_err(|e| anyhow::anyhow!("read specimen source {} failed: {e}", source.display()))?;
    load_specimens_from_markdown(db, &markdown, force)
}

pub fn list_specimens(db: &DbInstance, filter: Option<&str>) -> Result<Vec<SpecimenRecord>> {
    ensure_specimen_library_loaded(db, false)?;
    let mut records = read_all_specimens(db)?;
    if let Some(raw_filter) = filter {
        let (axis, expected) = parse_filter(raw_filter)?;
        records.retain(|record| specimen_field(record, &axis) == expected);
    }
    Ok(records)
}

pub fn parse_specimen_table(markdown: &str) -> Vec<SpecimenRecord> {
    let mut in_table = false;
    let mut records = Vec::new();

    for line in markdown.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("### Specimen Library") {
            in_table = true;
            continue;
        }
        if !in_table {
            continue;
        }
        if trimmed.starts_with("## ") {
            break;
        }
        if !trimmed.starts_with('|') || trimmed.contains("|---") {
            continue;
        }
        let cells: Vec<&str> = trimmed
            .trim_matches('|')
            .split('|')
            .map(str::trim)
            .collect();
        if cells.len() != 2 || cells[0] == "Situation type" {
            continue;
        }
        records.push(record_from_row(cells[0], cells[1]));
    }

    records
}

fn load_specimens_from_markdown(
    db: &DbInstance,
    markdown: &str,
    force: bool,
) -> Result<SpecimenLoadResult> {
    ensure_gametheory_schema(db)?;
    if force {
        db.run_script(
            "{::remove gt_specimen_library}",
            Default::default(),
            ScriptMutability::Mutable,
        )
        .map_err(|e| anyhow::anyhow!("remove old gt_specimen_library failed: {e}"))?;
        ensure_gametheory_schema(db)?;
    }

    let records = parse_specimen_table(markdown);
    for record in &records {
        insert_specimen(db, record)?;
    }

    Ok(SpecimenLoadResult {
        inserted: records.len(),
        total: count_specimens(db)?,
    })
}

fn record_from_row(situation_type: &str, classification: &str) -> SpecimenRecord {
    let tokens: Vec<String> = classification
        .split(',')
        .map(normalize_axis_value)
        .filter(|token| !token.is_empty())
        .collect();

    let cooperation = find_token(&tokens, &["non-cooperative", "cooperative"]);
    let payoff_sum = find_token(&tokens, &["non-zero-sum", "zero-sum", "constant-sum"]);
    let symmetry = find_token(&tokens, &["asymmetric", "symmetric"]);
    let timing = find_token(&tokens, &["simultaneous", "sequential", "mixed"]);
    let perfect_info = find_token(&tokens, &["imperfect info", "perfect info"]);
    let complete_info = find_token(&tokens, &["incomplete info", "complete info"]);
    let cardinality = find_token(&tokens, &["infinite", "finite players", "finite"]);
    let strategy_space = find_token(&tokens, &["continuous", "discrete"]);
    let horizon = find_token(&tokens, &["one-shot", "repeated"]);

    SpecimenRecord {
        specimen_id: stable_specimen_id(situation_type),
        situation_type: situation_type.to_string(),
        cooperation,
        payoff_sum,
        symmetry,
        timing,
        perfect_info,
        complete_info,
        cardinality,
        strategy_space,
        horizon,
        primary_family: situation_type.to_string(),
        notes: classification.to_string(),
    }
}

fn normalize_axis_value(value: &str) -> String {
    let mut normalized = value
        .trim()
        .trim_matches('*')
        .to_lowercase()
        .replace(" for group", "")
        .replace("continuous bids", "continuous")
        .replace("discrete options", "discrete");

    if let Some((before_note, _)) = normalized.split_once(" (") {
        normalized = before_note.to_string();
    }

    match normalized.as_str() {
        "imperfect" => "imperfect info".to_string(),
        "perfect" => "perfect info".to_string(),
        "incomplete" => "incomplete info".to_string(),
        "complete" => "complete info".to_string(),
        "finite players" => "finite".to_string(),
        other => other.to_string(),
    }
}

fn find_token(tokens: &[String], needles: &[&str]) -> String {
    for needle in needles {
        if let Some(token) = tokens.iter().find(|token| token.contains(needle)) {
            return token.clone();
        }
    }
    "unknown".to_string()
}

fn stable_specimen_id(situation_type: &str) -> String {
    let slug: String = situation_type
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    let compact = slug
        .split('-')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    format!("specimen-{compact}")
}

fn insert_specimen(db: &DbInstance, record: &SpecimenRecord) -> Result<()> {
    let mut params = BTreeMap::new();
    params.insert("id".into(), DataValue::from(record.specimen_id.as_str()));
    params.insert("sit".into(), DataValue::from(record.situation_type.as_str()));
    params.insert("coop".into(), DataValue::from(record.cooperation.as_str()));
    params.insert("pay".into(), DataValue::from(record.payoff_sum.as_str()));
    params.insert("sym".into(), DataValue::from(record.symmetry.as_str()));
    params.insert("timing".into(), DataValue::from(record.timing.as_str()));
    params.insert("perfect".into(), DataValue::from(record.perfect_info.as_str()));
    params.insert("complete".into(), DataValue::from(record.complete_info.as_str()));
    params.insert("card".into(), DataValue::from(record.cardinality.as_str()));
    params.insert("space".into(), DataValue::from(record.strategy_space.as_str()));
    params.insert("horizon".into(), DataValue::from(record.horizon.as_str()));
    params.insert("family".into(), DataValue::from(record.primary_family.as_str()));
    params.insert("notes".into(), DataValue::from(record.notes.as_str()));

    db.run_script(
        "?[specimen_id, situation_type, cooperation, payoff_sum, symmetry, timing, \
         perfect_info, complete_info, cardinality, strategy_space, horizon, primary_family, notes] \
         <- [[$id, $sit, $coop, $pay, $sym, $timing, $perfect, $complete, $card, $space, \
         $horizon, $family, $notes]] \
         :put gt_specimen_library { specimen_id => situation_type, cooperation, payoff_sum, \
         symmetry, timing, perfect_info, complete_info, cardinality, strategy_space, horizon, \
         primary_family, notes }",
        params,
        ScriptMutability::Mutable,
    )
    .map_err(|e| anyhow::anyhow!("insert gt_specimen_library failed: {e}"))?;
    Ok(())
}

fn read_all_specimens(db: &DbInstance) -> Result<Vec<SpecimenRecord>> {
    let rows = db
        .run_script(
            "?[specimen_id, situation_type, cooperation, payoff_sum, symmetry, timing, \
             perfect_info, complete_info, cardinality, strategy_space, horizon, primary_family, notes] \
             := *gt_specimen_library{specimen_id, situation_type, cooperation, payoff_sum, \
             symmetry, timing, perfect_info, complete_info, cardinality, strategy_space, horizon, \
             primary_family, notes}",
            Default::default(),
            ScriptMutability::Immutable,
        )
        .map_err(|e| anyhow::anyhow!("query gt_specimen_library failed: {e}"))?;

    let mut records = Vec::new();
    for row in rows.rows {
        records.push(SpecimenRecord {
            specimen_id: str_col(&row, 0),
            situation_type: str_col(&row, 1),
            cooperation: str_col(&row, 2),
            payoff_sum: str_col(&row, 3),
            symmetry: str_col(&row, 4),
            timing: str_col(&row, 5),
            perfect_info: str_col(&row, 6),
            complete_info: str_col(&row, 7),
            cardinality: str_col(&row, 8),
            strategy_space: str_col(&row, 9),
            horizon: str_col(&row, 10),
            primary_family: str_col(&row, 11),
            notes: str_col(&row, 12),
        });
    }
    records.sort_by(|a, b| a.situation_type.cmp(&b.situation_type));
    Ok(records)
}

fn str_col(row: &[DataValue], index: usize) -> String {
    row.get(index)
        .and_then(DataValue::get_str)
        .unwrap_or("")
        .to_string()
}

fn count_specimens(db: &DbInstance) -> Result<usize> {
    let result = db
        .run_script(
            "?[count(specimen_id)] := *gt_specimen_library{specimen_id}",
            Default::default(),
            ScriptMutability::Immutable,
        )
        .map_err(|e| anyhow::anyhow!("count gt_specimen_library failed: {e}"))?;
    Ok(result
        .rows
        .first()
        .and_then(|row| row[0].get_int())
        .unwrap_or(0) as usize)
}

fn parse_filter(raw_filter: &str) -> Result<(String, String)> {
    let Some((axis, value)) = raw_filter.split_once('=') else {
        anyhow::bail!("specimen filter must use axis=value format");
    };
    let axis = axis.trim().to_lowercase();
    let value = value.trim().to_lowercase();
    if axis.is_empty() || value.is_empty() {
        anyhow::bail!("specimen filter axis and value must be non-empty");
    }
    if !is_supported_axis(&axis) {
        anyhow::bail!("unsupported specimen filter axis: {axis}");
    }
    Ok((axis, value))
}

fn is_supported_axis(axis: &str) -> bool {
    matches!(
        axis,
        "cooperation"
            | "payoff_sum"
            | "symmetry"
            | "timing"
            | "perfect_info"
            | "complete_info"
            | "cardinality"
            | "strategy_space"
            | "horizon"
            | "primary_family"
    )
}

fn specimen_field(record: &SpecimenRecord, axis: &str) -> String {
    match axis {
        "cooperation" => record.cooperation.clone(),
        "payoff_sum" => record.payoff_sum.clone(),
        "symmetry" => record.symmetry.clone(),
        "timing" => record.timing.clone(),
        "perfect_info" => record.perfect_info.clone(),
        "complete_info" => record.complete_info.clone(),
        "cardinality" => record.cardinality.clone(),
        "strategy_space" => record.strategy_space.clone(),
        "horizon" => record.horizon.clone(),
        "primary_family" => record.primary_family.to_lowercase(),
        _ => String::new(),
    }
}

fn resolve_specimen_source_path() -> Result<PathBuf> {
    if let Some(path) = find_from_current_dir()? {
        return Ok(path);
    }

    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    for ancestor in manifest.ancestors() {
        let candidate = ancestor.join(SPECIMEN_SOURCE_RELATIVE);
        if candidate.exists() {
            return Ok(candidate);
        }
    }

    anyhow::bail!("could not find {SPECIMEN_SOURCE_RELATIVE}");
}

fn find_from_current_dir() -> Result<Option<PathBuf>> {
    let current = std::env::current_dir()?;
    for ancestor in current.ancestors() {
        let candidate = ancestor.join(SPECIMEN_SOURCE_RELATIVE);
        if candidate.exists() {
            return Ok(Some(candidate));
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_db() -> DbInstance {
        let path = format!("/tmp/test-gt-specimens-{}.db", uuid::Uuid::new_v4());
        DbInstance::new("sqlite", &path, "").unwrap()
    }

    fn source_markdown() -> String {
        std::fs::read_to_string(resolve_specimen_source_path().unwrap()).unwrap()
    }

    #[test]
    fn test_specimen_library_parses_readme_table() {
        let records = parse_specimen_table(&source_markdown());
        assert_eq!(records.len(), 6);

        let poker = records
            .iter()
            .find(|record| record.situation_type == "Poker hand")
            .expect("poker specimen must be parsed");
        assert_eq!(poker.cooperation, "non-cooperative");
        assert_eq!(poker.payoff_sum, "zero-sum");
        assert_eq!(poker.timing, "sequential");
        assert_eq!(poker.perfect_info, "imperfect info");
        assert_eq!(poker.complete_info, "incomplete info");
    }

    #[test]
    fn test_specimen_filter_by_axis() {
        let db = test_db();
        let loaded = load_specimens_from_markdown(&db, &source_markdown(), true).unwrap();
        assert_eq!(loaded.total, 6);

        let cooperative = list_specimens(&db, Some("cooperation=cooperative")).unwrap();
        assert_eq!(cooperative.len(), 1);
        assert_eq!(cooperative[0].situation_type, "Climate treaty negotiations");

        let simultaneous = list_specimens(&db, Some("timing=simultaneous")).unwrap();
        assert_eq!(simultaneous.len(), 2);
    }

    #[test]
    fn test_specimen_lazy_load_on_first_use() {
        let db = test_db();
        ensure_gametheory_schema(&db).unwrap();
        assert_eq!(count_specimens(&db).unwrap(), 0);

        let records = list_specimens(&db, None).unwrap();
        assert_eq!(records.len(), 6);

        let stored_count = count_specimens(&db).unwrap();
        assert_eq!(stored_count, 6);

        let loaded_again = ensure_specimen_library_loaded(&db, false).unwrap();
        assert_eq!(loaded_again.inserted, 0);
        assert_eq!(loaded_again.total, 6);
    }

    #[test]
    fn test_specimen_filter_rejects_invalid_format() {
        let db = test_db();
        let err = list_specimens(&db, Some("cooperation")).unwrap_err();
        assert!(err.to_string().contains("axis=value"));
    }
}
