#[derive(Debug, Clone, Copy)]
struct TrainingSchedule {
    candidate_count: u64,
    new_rows_since_training: u64,
    last_training_age_ms: Option<u64>,
}

#[derive(Debug, Clone, Copy)]
struct CandidateTrainingSummary {
    row_count: u64,
    created_at: chrono::DateTime<Utc>,
}

fn latent_training_schedule(
    root: &Path,
    total_rows: u64,
    candidate_count: u64,
    last_training_age_ms: Option<u64>,
) -> Result<TrainingSchedule> {
    let latest = latest_latent_candidate(root)?;
    Ok(schedule_from_latest(
        total_rows,
        candidate_count,
        latest,
        last_training_age_ms,
    ))
}

fn jepa_training_schedule(
    root: &Path,
    total_rows: u64,
    candidate_count: u64,
    last_training_age_ms: Option<u64>,
) -> Result<TrainingSchedule> {
    let latest = latest_jepa_candidate(root)?;
    Ok(schedule_from_latest(
        total_rows,
        candidate_count,
        latest,
        last_training_age_ms,
    ))
}

fn schedule_from_latest(
    total_rows: u64,
    candidate_count: u64,
    latest: Option<CandidateTrainingSummary>,
    last_training_age_ms: Option<u64>,
) -> TrainingSchedule {
    let trained_rows = latest.map(|candidate| candidate.row_count).unwrap_or(0);
    let derived_age = latest.and_then(|candidate| age_ms_since(candidate.created_at));
    TrainingSchedule {
        candidate_count,
        new_rows_since_training: if latest.is_some() {
            total_rows.saturating_sub(trained_rows)
        } else {
            total_rows
        },
        last_training_age_ms: last_training_age_ms.or(derived_age),
    }
}

fn latest_latent_candidate(root: &Path) -> Result<Option<CandidateTrainingSummary>> {
    let dir = root.join("candidates");
    if !dir.exists() {
        return Ok(None);
    }
    let mut latest = None;
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if path.extension().and_then(|ext| ext.to_str()) != Some("json")
            || name.ends_with(".eval.json")
        {
            continue;
        }
        let record: CpuCandidateRecord = serde_json::from_str(&std::fs::read_to_string(path)?)?;
        update_latest(
            &mut latest,
            CandidateTrainingSummary {
                row_count: record.model.metadata.row_count,
                created_at: record.created_at,
            },
        );
    }
    Ok(latest)
}

fn latest_jepa_candidate(root: &Path) -> Result<Option<CandidateTrainingSummary>> {
    let dir = root.join("jepa").join("candidates");
    if !dir.exists() {
        return Ok(None);
    }
    let mut latest = None;
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        let record: JepaCandidateRecord = serde_json::from_str(&std::fs::read_to_string(path)?)?;
        update_latest(
            &mut latest,
            CandidateTrainingSummary {
                row_count: record.model.metadata.row_count,
                created_at: record.created_at,
            },
        );
    }
    Ok(latest)
}

fn update_latest(
    latest: &mut Option<CandidateTrainingSummary>,
    candidate: CandidateTrainingSummary,
) {
    if latest
        .as_ref()
        .is_none_or(|current| candidate.created_at > current.created_at)
    {
        *latest = Some(candidate);
    }
}

fn age_ms_since(created_at: chrono::DateTime<Utc>) -> Option<u64> {
    let millis = Utc::now().signed_duration_since(created_at).num_milliseconds();
    Some(millis.max(0) as u64)
}
