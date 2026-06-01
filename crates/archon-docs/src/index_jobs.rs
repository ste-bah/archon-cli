use std::collections::BTreeMap;

use anyhow::Result;
use cozo::{DataValue, DbInstance, ScriptMutability};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct IndexJobSummary {
    pub running: usize,
    pub paused: usize,
    pub completed: usize,
    pub failed: usize,
    pub cancelled: usize,
}

#[derive(Clone, Debug)]
pub struct IndexJobRecord {
    pub job_id: String,
    pub scope: String,
    pub document_id: String,
    pub provider: String,
    pub dimension: i64,
    pub status: String,
    pub started_at: String,
    pub completed_at: String,
    pub leased: i64,
    pub indexed: i64,
    pub failed: i64,
    pub skipped: i64,
    pub last_error: String,
}

pub fn start_job(
    db: &DbInstance,
    scope: &str,
    document_id: Option<&str>,
    provider: &str,
    dimension: usize,
) -> Result<String> {
    let job = IndexJobRecord {
        job_id: format!("idx-{}", uuid::Uuid::new_v4()),
        scope: scope.into(),
        document_id: document_id.unwrap_or("").into(),
        provider: provider.into(),
        dimension: dimension as i64,
        status: "running".into(),
        started_at: now(),
        completed_at: String::new(),
        leased: 0,
        indexed: 0,
        failed: 0,
        skipped: 0,
        last_error: String::new(),
    };
    put_job(db, &job)?;
    Ok(job.job_id)
}

pub fn record_progress(
    db: &DbInstance,
    job_id: &str,
    leased: usize,
    indexed: usize,
    failed: usize,
    skipped: usize,
) -> Result<()> {
    let Some(mut job) = get_job(db, job_id)? else {
        return Ok(());
    };
    job.leased += leased as i64;
    job.indexed += indexed as i64;
    job.failed += failed as i64;
    job.skipped += skipped as i64;
    put_job(db, &job)
}

pub fn finish_job(db: &DbInstance, job_id: &str, error: Option<&str>) -> Result<()> {
    let Some(mut job) = get_job(db, job_id)? else {
        return Ok(());
    };
    if matches!(job.status.as_str(), "paused" | "cancelled") {
        return Ok(());
    }
    job.status = if error.is_some() {
        "failed".into()
    } else {
        "completed".into()
    };
    job.completed_at = now();
    job.last_error = error.unwrap_or("").chars().take(500).collect();
    put_job(db, &job)
}

pub fn summary(db: &DbInstance) -> Result<IndexJobSummary> {
    Ok(IndexJobSummary {
        running: count_status(db, "running")?,
        paused: count_status(db, "paused")?,
        completed: count_status(db, "completed")?,
        failed: count_status(db, "failed")?,
        cancelled: count_status(db, "cancelled")?,
    })
}

pub fn list_recent(db: &DbInstance, limit: usize) -> Result<Vec<IndexJobRecord>> {
    let script = format!(
        "?[job_id, scope, document_id, provider, dimension, status, started_at, completed_at, leased, indexed, failed, skipped, last_error] \
         := *doc_index_jobs{{job_id, scope, document_id, provider, dimension, status, started_at, completed_at, leased, indexed, failed, skipped, last_error}} \
         :order -started_at :limit {}",
        limit.max(1)
    );
    let result = crate::cozo_retry::run_script_guarded(
        db,
        &script,
        BTreeMap::new(),
        ScriptMutability::Immutable,
        "list recent index jobs",
    )
    .map_err(|e| anyhow::anyhow!("list recent index jobs failed: {e}"))?;
    Ok(result.rows.iter().map(|row| job_from_row(row)).collect())
}

pub fn pause_job(db: &DbInstance, job_id: &str) -> Result<()> {
    set_job_status(db, job_id, "paused", None)
}

pub fn resume_job(db: &DbInstance, job_id: &str) -> Result<()> {
    set_job_status(db, job_id, "running", None)
}

pub fn cancel_job(db: &DbInstance, job_id: &str) -> Result<()> {
    set_job_status(db, job_id, "cancelled", Some("cancelled by user"))
}

pub fn control_status(db: &DbInstance, job_id: &str) -> Result<Option<String>> {
    Ok(get_job(db, job_id)?.and_then(|job| {
        matches!(job.status.as_str(), "paused" | "cancelled").then_some(job.status)
    }))
}

fn set_job_status(db: &DbInstance, job_id: &str, status: &str, error: Option<&str>) -> Result<()> {
    let Some(mut job) = get_job(db, job_id)? else {
        anyhow::bail!("index job not found: {job_id}");
    };
    job.status = status.into();
    if matches!(status, "cancelled" | "failed" | "completed") {
        job.completed_at = now();
    }
    job.last_error = error.unwrap_or("").chars().take(500).collect();
    put_job(db, &job)
}

fn get_job(db: &DbInstance, job_id: &str) -> Result<Option<IndexJobRecord>> {
    let mut params = BTreeMap::new();
    params.insert("job_id".into(), DataValue::from(job_id));
    let result = crate::cozo_retry::run_script_guarded(
            db,
            "?[job_id, scope, document_id, provider, dimension, status, started_at, completed_at, leased, indexed, failed, skipped, last_error] \
             := *doc_index_jobs{job_id, scope, document_id, provider, dimension, status, started_at, completed_at, leased, indexed, failed, skipped, last_error}, \
             job_id = $job_id",
            params,
            ScriptMutability::Immutable,
            "get index job",
        )
        .map_err(|e| anyhow::anyhow!("get index job failed: {e}"))?;
    Ok(result.rows.first().map(|row| job_from_row(row)))
}

fn put_job(db: &DbInstance, job: &IndexJobRecord) -> Result<()> {
    let mut params = BTreeMap::new();
    params.insert("job_id".into(), DataValue::from(job.job_id.as_str()));
    params.insert("scope".into(), DataValue::from(job.scope.as_str()));
    params.insert(
        "document_id".into(),
        DataValue::from(job.document_id.as_str()),
    );
    params.insert("provider".into(), DataValue::from(job.provider.as_str()));
    params.insert("dimension".into(), DataValue::from(job.dimension));
    params.insert("status".into(), DataValue::from(job.status.as_str()));
    params.insert(
        "started_at".into(),
        DataValue::from(job.started_at.as_str()),
    );
    params.insert(
        "completed_at".into(),
        DataValue::from(job.completed_at.as_str()),
    );
    params.insert("leased".into(), DataValue::from(job.leased));
    params.insert("indexed".into(), DataValue::from(job.indexed));
    params.insert("failed".into(), DataValue::from(job.failed));
    params.insert("skipped".into(), DataValue::from(job.skipped));
    params.insert(
        "last_error".into(),
        DataValue::from(job.last_error.as_str()),
    );
    crate::cozo_retry::run_script_guarded(
        db,
        "?[job_id, scope, document_id, provider, dimension, status, started_at, completed_at, leased, indexed, failed, skipped, last_error] \
         <- [[$job_id, $scope, $document_id, $provider, $dimension, $status, $started_at, $completed_at, $leased, $indexed, $failed, $skipped, $last_error]]
         :put doc_index_jobs { job_id => scope, document_id, provider, dimension, status, started_at, completed_at, leased, indexed, failed, skipped, last_error }",
        params,
        ScriptMutability::Mutable,
        "write index job",
    )
    .map_err(|e| anyhow::anyhow!("write index job failed: {e}"))?;
    Ok(())
}

fn count_status(db: &DbInstance, status: &str) -> Result<usize> {
    let mut params = BTreeMap::new();
    params.insert("status".into(), DataValue::from(status));
    let result = crate::cozo_retry::run_script_guarded(
        db,
        "?[count(job_id)] := *doc_index_jobs{job_id, status}, status = $status",
        params,
        ScriptMutability::Immutable,
        "count index jobs",
    )
    .map_err(|e| anyhow::anyhow!("count index jobs failed: {e}"))?;
    Ok(result.rows[0][0].get_int().unwrap_or(0) as usize)
}

fn job_from_row(row: &[DataValue]) -> IndexJobRecord {
    IndexJobRecord {
        job_id: row[0].get_str().unwrap_or("").to_string(),
        scope: row[1].get_str().unwrap_or("").to_string(),
        document_id: row[2].get_str().unwrap_or("").to_string(),
        provider: row[3].get_str().unwrap_or("").to_string(),
        dimension: row[4].get_int().unwrap_or(0),
        status: row[5].get_str().unwrap_or("").to_string(),
        started_at: row[6].get_str().unwrap_or("").to_string(),
        completed_at: row[7].get_str().unwrap_or("").to_string(),
        leased: row[8].get_int().unwrap_or(0),
        indexed: row[9].get_int().unwrap_or(0),
        failed: row[10].get_int().unwrap_or(0),
        skipped: row[11].get_int().unwrap_or(0),
        last_error: row[12].get_str().unwrap_or("").to_string(),
    }
}

fn now() -> String {
    chrono::Utc::now().to_rfc3339()
}

#[cfg(test)]
mod tests {
    use cozo::DbInstance;

    use super::*;
    use crate::schema::ensure_doc_schema;

    #[test]
    fn job_lifecycle_updates_summary() {
        let db = DbInstance::new("mem", "", Default::default()).unwrap();
        ensure_doc_schema(&db).unwrap();
        let job_id = start_job(&db, "pending", None, "test", 3).unwrap();
        record_progress(&db, &job_id, 2, 1, 1, 0).unwrap();
        assert_eq!(summary(&db).unwrap().running, 1);
        finish_job(&db, &job_id, None).unwrap();
        let summary = summary(&db).unwrap();
        assert_eq!(summary.running, 0);
        assert_eq!(summary.completed, 1);
    }
}
