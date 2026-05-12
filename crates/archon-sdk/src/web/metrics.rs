use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use archon_learning::runtime_models::ProviderRuntimeEventRecord;
use axum::{
    Json,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};
use ts_rs::{Config as TsConfig, TS};

use super::{AppState, check_auth, inspect::PathProbe};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct MetricsSummary {
    pub logs: Vec<PathProbe>,
    pub budgets: Vec<PathProbe>,
    pub web_bundle_files: u64,
    pub web_bundle_bytes: u64,
    pub stores: Vec<MetricStoreHealth>,
    pub performance: Vec<MetricValue>,
    pub queues: Vec<MetricValue>,
    pub recent_events: Vec<MetricEventPreview>,
    pub provider_metrics: Vec<ProviderRuntimeMetric>,
    pub provider_events: Vec<ProviderRuntimeEventPreview>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct MetricStoreHealth {
    pub label: String,
    pub status: String,
    pub path: String,
    pub files: u64,
    pub bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct MetricValue {
    pub label: String,
    pub value: String,
    pub unit: String,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct MetricEventPreview {
    pub source: String,
    pub summary: String,
    pub severity: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct ProviderRuntimeMetric {
    pub provider_id: String,
    pub request_count: u64,
    pub error_count: u64,
    pub retry_count: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub estimated_cost_usd: f64,
    pub latency_ms_p95: u64,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct ProviderRuntimeEventPreview {
    pub provider_id: String,
    pub model_id: String,
    pub event_type: String,
    pub severity: String,
    pub message: String,
    pub created_at: String,
}

pub(crate) async fn summary_handler(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(resp) = check_auth(&state, &headers) {
        return resp;
    }
    (StatusCode::OK, Json(metrics_summary())).into_response()
}

fn metrics_summary() -> MetricsSummary {
    let cwd = cwd();
    let home = home_archon();
    let dist_probe = probe("web dist", cwd.join("web/dist"));
    let provider_events = provider_runtime_records(&cwd);
    MetricsSummary {
        logs: log_probes(&home),
        budgets: vec![probe("budget records", home.join("budget"))],
        web_bundle_files: dist_probe.files,
        web_bundle_bytes: dist_probe.bytes,
        stores: store_health(&home, &cwd),
        performance: performance_values(&dist_probe),
        queues: queue_values(&home),
        recent_events: recent_events(&home, 8),
        provider_metrics: provider_metrics(&provider_events),
        provider_events: provider_event_previews(&provider_events, 8),
    }
}

fn log_probes(home: &Path) -> Vec<PathProbe> {
    vec![
        probe("system logs", home.join("logs")),
        probe("web action audit", home.join("web/actions.audit.jsonl")),
        probe(
            "reasoning quality events",
            home.join("reasoning-quality/events.jsonl"),
        ),
        probe(
            "world advisor events",
            home.join("world-model/ledgers/world-advisor-events.jsonl"),
        ),
    ]
}

fn store_health(home: &Path, cwd: &Path) -> Vec<MetricStoreHealth> {
    [
        probe("sessions", home.join("sessions")),
        probe("memory", home.join("memory")),
        probe("world model", home.join("world-model")),
        probe("reasoning quality", home.join("reasoning-quality")),
        probe("web dist", cwd.join("web/dist")),
    ]
    .into_iter()
    .map(|probe| MetricStoreHealth {
        status: if probe.exists { "ready" } else { "missing" }.into(),
        label: probe.label,
        path: probe.path,
        files: probe.files,
        bytes: probe.bytes,
    })
    .collect()
}

fn performance_values(dist: &PathProbe) -> Vec<MetricValue> {
    vec![
        metric(
            "Initial JS budget",
            bytes_label(dist.bytes),
            "< 1.5 MB gzip",
            if dist.bytes < 1_500_000 {
                "good"
            } else {
                "warn"
            },
        ),
        metric("Tab switch target", "150".into(), "ms", "tracked"),
        metric("Live event target", "250".into(), "ms", "tracked"),
        metric("Corpus search target", "1000".into(), "ms", "tracked"),
    ]
}

fn queue_values(home: &Path) -> Vec<MetricValue> {
    vec![
        queue("web action audit", home.join("web/actions.audit.jsonl")),
        queue(
            "reasoning events",
            home.join("reasoning-quality/events.jsonl"),
        ),
        queue(
            "world advisor events",
            home.join("world-model/ledgers/world-advisor-events.jsonl"),
        ),
    ]
}

fn queue(label: &str, path: PathBuf) -> MetricValue {
    let lines = count_lines(&path);
    metric(
        label,
        lines.to_string(),
        "rows",
        if lines > 0 { "active" } else { "quiet" },
    )
}

fn recent_events(home: &Path, limit: usize) -> Vec<MetricEventPreview> {
    let ledgers = [
        ("web", home.join("web/actions.audit.jsonl")),
        ("reasoning", home.join("reasoning-quality/events.jsonl")),
        (
            "world",
            home.join("world-model/ledgers/world-advisor-events.jsonl"),
        ),
    ];
    ledgers
        .iter()
        .flat_map(|(source, path)| read_recent_lines(source, path, limit))
        .take(limit)
        .collect()
}

fn provider_runtime_records(cwd: &Path) -> Vec<ProviderRuntimeEventRecord> {
    learning_db_candidates(cwd)
        .into_iter()
        .find_map(|path| {
            if !path.exists() {
                return None;
            }
            let path_str = path.to_string_lossy().to_string();
            let db = cozo::DbInstance::new("sqlite", &path_str, "").ok()?;
            archon_learning::runtime_events::list_provider_runtime_events(&db, None).ok()
        })
        .unwrap_or_default()
}

fn learning_db_candidates(cwd: &Path) -> Vec<PathBuf> {
    let mut paths = vec![cwd.join(".archon/learning.db")];
    if let Some(parent) = cwd.parent() {
        paths.push(parent.join("learning.db"));
    }
    paths.push(home_archon().join("learning.db"));
    paths
}

fn provider_metrics(records: &[ProviderRuntimeEventRecord]) -> Vec<ProviderRuntimeMetric> {
    let mut metrics = BTreeMap::<String, ProviderRuntimeMetric>::new();
    let mut latencies = BTreeMap::<String, Vec<u64>>::new();
    for record in records {
        let entry = metrics
            .entry(record.provider_id.clone())
            .or_insert_with(|| ProviderRuntimeMetric {
                provider_id: record.provider_id.clone(),
                request_count: 0,
                error_count: 0,
                retry_count: 0,
                input_tokens: 0,
                output_tokens: 0,
                estimated_cost_usd: 0.0,
                latency_ms_p95: 0,
                status: "ok".into(),
            });
        entry.request_count += 1;
        entry.retry_count += record.retry_count.unwrap_or(0) as u64;
        entry.input_tokens += usage_count(record, "input_count");
        entry.output_tokens += usage_count(record, "output_count");
        entry.estimated_cost_usd += cost_usd(record);
        if is_error_event(record) {
            entry.error_count += 1;
            entry.status = "warn".into();
        }
        if let Some(latency) = latency_ms(record) {
            latencies
                .entry(record.provider_id.clone())
                .or_default()
                .push(latency);
        }
    }
    for (provider, samples) in latencies {
        if let Some(metric) = metrics.get_mut(&provider) {
            metric.latency_ms_p95 = percentile_95(samples);
        }
    }
    metrics.into_values().collect()
}

fn provider_event_previews(
    records: &[ProviderRuntimeEventRecord],
    limit: usize,
) -> Vec<ProviderRuntimeEventPreview> {
    records
        .iter()
        .take(limit)
        .map(|record| ProviderRuntimeEventPreview {
            provider_id: record.provider_id.clone(),
            model_id: record.model_id.clone().unwrap_or_else(|| "unknown".into()),
            event_type: record.event_type.clone(),
            severity: record.severity.clone(),
            message: record
                .message
                .clone()
                .or_else(|| record.reason_code.clone())
                .unwrap_or_else(|| "provider runtime event".into()),
            created_at: record.created_at.clone(),
        })
        .collect()
}

fn usage_count(record: &ProviderRuntimeEventRecord, key: &str) -> u64 {
    record
        .raw_redacted_json
        .get("usage")
        .and_then(|usage| usage.get(key))
        .and_then(|value| value.as_u64())
        .unwrap_or(0)
}

fn cost_usd(record: &ProviderRuntimeEventRecord) -> f64 {
    record
        .raw_redacted_json
        .get("cost_usd")
        .or_else(|| record.raw_redacted_json.pointer("/cost/usd"))
        .and_then(|value| value.as_f64())
        .unwrap_or(0.0)
}

fn latency_ms(record: &ProviderRuntimeEventRecord) -> Option<u64> {
    record
        .raw_redacted_json
        .get("latency_ms")
        .or_else(|| record.raw_redacted_json.get("duration_ms"))
        .and_then(|value| value.as_u64())
}

fn is_error_event(record: &ProviderRuntimeEventRecord) -> bool {
    record.severity == "error"
        || record.severity == "warn"
        || record.event_type.contains("failed")
        || record.event_type.contains("error")
}

fn percentile_95(mut samples: Vec<u64>) -> u64 {
    if samples.is_empty() {
        return 0;
    }
    samples.sort_unstable();
    let index = ((samples.len() as f64 - 1.0) * 0.95).round() as usize;
    samples[index]
}

fn read_recent_lines(source: &str, path: &Path, limit: usize) -> Vec<MetricEventPreview> {
    let Ok(text) = fs::read_to_string(path) else {
        return Vec::new();
    };
    text.lines()
        .rev()
        .take(limit)
        .map(|line| MetricEventPreview {
            source: source.into(),
            summary: line.chars().take(160).collect(),
            severity: if line.contains("failed") || line.contains("unavailable") {
                "warn".into()
            } else {
                "info".into()
            },
            created_at: "ledger tail".into(),
        })
        .collect()
}

fn metric(label: &str, value: String, unit: &str, status: &str) -> MetricValue {
    MetricValue {
        label: label.into(),
        value,
        unit: unit.into(),
        status: status.into(),
    }
}

fn count_lines(path: &Path) -> u64 {
    fs::read_to_string(path)
        .map(|text| text.lines().count() as u64)
        .unwrap_or(0)
}

fn probe(label: impl Into<String>, path: PathBuf) -> PathProbe {
    let (files, bytes) = dir_stats(&path, 0);
    PathProbe {
        label: label.into(),
        path: display_path(&path),
        exists: path.exists(),
        files,
        bytes,
    }
}

fn dir_stats(path: &Path, depth: usize) -> (u64, u64) {
    if depth > 3 {
        return (0, 0);
    }
    let Ok(metadata) = fs::metadata(path) else {
        return (0, 0);
    };
    if metadata.is_file() {
        return (1, metadata.len());
    }
    fs::read_dir(path)
        .ok()
        .into_iter()
        .flatten()
        .flatten()
        .fold((0, 0), |(files, bytes), entry| {
            let (child_files, child_bytes) = dir_stats(&entry.path(), depth + 1);
            (files + child_files, bytes + child_bytes)
        })
}

fn bytes_label(value: u64) -> String {
    if value < 1024 {
        format!("{value} B")
    } else if value < 1024 * 1024 {
        format!("{} KB", value / 1024)
    } else {
        format!("{:.1} MB", value as f64 / (1024.0 * 1024.0))
    }
}

fn cwd() -> PathBuf {
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

fn home_archon() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".archon")
}

fn display_path(path: &Path) -> String {
    path.to_string_lossy().to_string()
}

pub fn generated_typescript() -> String {
    let cfg = TsConfig::default().with_large_int("number");
    [
        exported(MetricsSummary::decl(&cfg)),
        exported(MetricStoreHealth::decl(&cfg)),
        exported(MetricValue::decl(&cfg)),
        exported(MetricEventPreview::decl(&cfg)),
        exported(ProviderRuntimeMetric::decl(&cfg)),
        exported(ProviderRuntimeEventPreview::decl(&cfg)),
    ]
    .join("\n\n")
        + "\n"
}

fn exported(decl: String) -> String {
    format!("export {decl}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_event_ledger_is_empty() {
        let events = read_recent_lines("web", Path::new("/not/real/events.jsonl"), 4);
        assert!(events.is_empty());
    }

    #[test]
    fn provider_metrics_aggregate_usage_and_errors() {
        let first = ProviderRuntimeEventRecord::new(
            "event-1",
            "anthropic",
            "direct",
            "request_succeeded",
            "info",
            "now",
        )
        .with_model("claude")
        .with_redacted_json(serde_json::json!({
            "usage": { "input_count": 100, "output_count": 40 },
            "latency_ms": 250,
            "cost_usd": 0.002
        }));
        let second = ProviderRuntimeEventRecord::new(
            "event-2",
            "anthropic",
            "direct",
            "request_failed",
            "warn",
            "later",
        )
        .with_retry_count(2)
        .with_redacted_json(serde_json::json!({
            "usage": { "input_count": 20, "output_count": 0 },
            "latency_ms": 800
        }));
        let metrics = provider_metrics(&[first, second]);
        assert_eq!(metrics.len(), 1);
        assert_eq!(metrics[0].request_count, 2);
        assert_eq!(metrics[0].error_count, 1);
        assert_eq!(metrics[0].retry_count, 2);
        assert_eq!(metrics[0].input_tokens, 120);
        assert_eq!(metrics[0].output_tokens, 40);
        assert_eq!(metrics[0].latency_ms_p95, 800);
    }
}
