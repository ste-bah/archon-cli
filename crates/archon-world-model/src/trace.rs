use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ColdStartThresholds {
    pub min_rows: u64,
    pub min_sessions: u64,
    pub min_observed_days: u64,
}

impl Default for ColdStartThresholds {
    fn default() -> Self {
        Self {
            min_rows: 1_000,
            min_sessions: 50,
            min_observed_days: 7,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ColdStartStats {
    pub rows: u64,
    pub sessions: u64,
    pub observed_days: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ColdStartStatus {
    Ready,
    ColdStart {
        rows_needed: u64,
        sessions_needed: u64,
        days_needed: u64,
    },
}

pub fn evaluate_cold_start(
    stats: ColdStartStats,
    thresholds: ColdStartThresholds,
) -> ColdStartStatus {
    let rows_needed = thresholds.min_rows.saturating_sub(stats.rows);
    let sessions_needed = thresholds.min_sessions.saturating_sub(stats.sessions);
    let days_needed = thresholds
        .min_observed_days
        .saturating_sub(stats.observed_days);

    if rows_needed == 0 && sessions_needed == 0 && days_needed == 0 {
        ColdStartStatus::Ready
    } else {
        ColdStartStatus::ColdStart {
            rows_needed,
            sessions_needed,
            days_needed,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cold_start_reports_missing_thresholds() {
        let status = evaluate_cold_start(
            ColdStartStats {
                rows: 25,
                sessions: 2,
                observed_days: 1,
            },
            ColdStartThresholds::default(),
        );

        assert_eq!(
            status,
            ColdStartStatus::ColdStart {
                rows_needed: 975,
                sessions_needed: 48,
                days_needed: 6
            }
        );
    }

    #[test]
    fn cold_start_ready_when_all_thresholds_met() {
        let status = evaluate_cold_start(
            ColdStartStats {
                rows: 1_000,
                sessions: 50,
                observed_days: 7,
            },
            ColdStartThresholds::default(),
        );

        assert_eq!(status, ColdStartStatus::Ready);
    }
}
