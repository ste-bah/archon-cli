//! Read `threshold.toml` once at process startup and expose the per-bench
//! p95 ceiling. Used by every bench in this crate to assert against the
//! NFR-PERF gate values.

use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::OnceLock;

#[derive(Debug, Deserialize)]
struct BenchThreshold {
    p95_ms: u64,
    #[allow(dead_code)]
    reference: Option<String>,
}

#[derive(Debug)]
pub enum ThresholdError {
    Io(String),
    Parse(String),
    Missing(String),
}

impl std::fmt::Display for ThresholdError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ThresholdError::Io(m) => write!(f, "threshold.toml I/O error: {m}"),
            ThresholdError::Parse(m) => write!(f, "threshold.toml parse error: {m}"),
            ThresholdError::Missing(b) => write!(f, "threshold.toml has no [{b}] section"),
        }
    }
}

impl std::error::Error for ThresholdError {}

fn threshold_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("threshold.toml")
}

fn load_all() -> Result<BTreeMap<String, BenchThreshold>, ThresholdError> {
    let path = threshold_path();
    let text =
        std::fs::read_to_string(&path).map_err(|e| ThresholdError::Io(format!("{path:?}: {e}")))?;
    toml::from_str::<BTreeMap<String, BenchThreshold>>(&text)
        .map_err(|e| ThresholdError::Parse(e.to_string()))
}

fn cache() -> &'static BTreeMap<String, BenchThreshold> {
    static CACHE: OnceLock<BTreeMap<String, BenchThreshold>> = OnceLock::new();
    CACHE.get_or_init(|| load_all().expect("threshold.toml must load at bench startup"))
}

/// Look up the p95 ceiling (milliseconds) for a bench section name.
/// Panics if the section is missing — a missing entry is a build-time
/// bug, not a runtime condition.
pub fn get_p95_ms(bench_name: &str) -> u64 {
    cache()
        .get(bench_name)
        .map(|t| t.p95_ms)
        .unwrap_or_else(|| panic!("{}", ThresholdError::Missing(bench_name.to_string())))
}

/// Fallible variant for unit-testing the loader without panicking.
#[doc(hidden)]
pub fn try_get_p95_ms(bench_name: &str) -> Result<u64, ThresholdError> {
    let all = load_all()?;
    all.get(bench_name)
        .map(|t| t.p95_ms)
        .ok_or_else(|| ThresholdError::Missing(bench_name.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_known_bench_sections() {
        // threshold.toml ships with these three sections per the NFR contract.
        assert_eq!(get_p95_ms("task_submit"), 100);
        assert_eq!(get_p95_ms("discovery_scan"), 1000);
        assert_eq!(get_p95_ms("fanout_100"), 1000);
    }

    #[test]
    fn missing_section_errors_in_fallible_api() {
        let err = try_get_p95_ms("nonexistent_bench").unwrap_err();
        assert!(matches!(err, ThresholdError::Missing(_)), "got: {err}");
    }
}
