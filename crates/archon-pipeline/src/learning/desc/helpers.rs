use super::types::DescEpisode;
use cozo::DataValue;

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

pub(super) fn now_epoch() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// Convert a CozoDB result row to a `DescEpisode`.
///
/// Expected column order (join of desc_episodes + desc_episode_metadata):
/// 0: episode_id, 1: session_id, 2: description, 3: outcome, 4: reward,
/// 5: tags, 6: created_at, 7: task_type, 8: solution, 9: quality_score,
/// 10: trajectory_id, 11: updated_at
pub(super) fn row_to_episode(row: &[DataValue]) -> DescEpisode {
    let tags: Vec<String> = match &row[5] {
        DataValue::List(list) => list
            .iter()
            .filter_map(|v| v.get_str().map(|s| s.to_string()))
            .collect(),
        _ => vec![],
    };

    let trajectory_id = row[10]
        .get_str()
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());

    DescEpisode {
        episode_id: row[0].get_str().unwrap_or("").to_string(),
        session_id: row[1].get_str().unwrap_or("").to_string(),
        description: row[2].get_str().unwrap_or("").to_string(),
        outcome: row[3].get_str().unwrap_or("").to_string(),
        reward: row[4].get_float().unwrap_or(0.0),
        tags,
        created_at: row[6].get_int().unwrap_or(0) as u64,
        task_type: row[7].get_str().unwrap_or("").to_string(),
        solution: row[8].get_str().unwrap_or("").to_string(),
        quality_score: row[9].get_float().unwrap_or(0.0),
        trajectory_id,
        updated_at: row[11].get_int().unwrap_or(0) as u64,
    }
}
