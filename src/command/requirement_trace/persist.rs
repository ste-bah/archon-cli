//! CLI output and optional knowledge-store persistence for requirement trace.

use std::io::Write;
use std::path::Path;

use anyhow::Result;
use archon_knowledge::traceability::TraceReport;
use archon_knowledge::traceability::anchors::anchor_relation;
use archon_knowledge::traceability::requirements::requirement_entity_for;
use archon_knowledge::traceability::store::AnchorRecord;

use super::absolute;

pub(super) fn write_cli_report(writer: &mut impl Write, report: &str) -> Result<()> {
    writer.write_all(report.as_bytes())?;
    Ok(())
}

/// Write requirement entities and anchored edges into a knowledge store.
pub(super) fn persist(cwd: &Path, store_path: &Path, report: &TraceReport) -> Result<()> {
    let path = absolute(cwd, store_path);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let db = archon_cozo::open_sqlite_guarded(
        path.to_string_lossy().as_ref(),
        "open knowledge store for requirement trace persistence",
        &archon_cozo::CozoGuardConfig::for_db_path(&path),
    )
    .map_err(|e| anyhow::anyhow!("opening knowledge store at {}: {e}", path.display()))?;
    archon_knowledge::schema::ensure_knowledge_schema(&db)?;
    archon_knowledge::traceability::store::ensure_traceability_schema(&db)?;

    let now = chrono::Utc::now().to_rfc3339();
    for row in &report.rows {
        let entity = requirement_entity_for(&row.requirement_id, row.prd_line, &report.prd_path);
        archon_knowledge::store::insert_entity(&db, &entity)?;
        for verdict in &row.anchors {
            archon_knowledge::traceability::store::insert_anchor(
                &db,
                &anchor_relation(&verdict.anchor, &entity.entity_id),
                &AnchorRecord::from_anchor(&verdict.anchor, verdict.level, &now),
            )?;
        }
    }
    Ok(())
}
