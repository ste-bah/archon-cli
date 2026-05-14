use std::{
    collections::{BTreeMap, BTreeSet},
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

use super::{AppState, WebRuntimePaths, assets, check_auth, inspect::PathProbe};

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
    (StatusCode::OK, Json(metrics_summary(&state.paths))).into_response()
}

fn metrics_summary(paths: &WebRuntimePaths) -> MetricsSummary {
    let dist_probe = embedded_asset_probe();
    let provider_events = provider_runtime_records(&paths.cwd, paths);
    MetricsSummary {
        logs: log_probes(paths),
        budgets: vec![probe("budget records", paths.archon_data.join("budget"))],
        web_bundle_files: dist_probe.files,
        web_bundle_bytes: dist_probe.bytes,
        stores: store_health(paths, &dist_probe),
        performance: performance_values(&dist_probe),
        queues: ledger_values(paths),
        recent_events: recent_events(&paths.archon_home, 8),
        provider_metrics: provider_metrics(&provider_events),
        provider_events: provider_event_previews(&provider_events, 8),
    }
}

fn log_probes(paths: &WebRuntimePaths) -> Vec<PathProbe> {
    vec![
        probe("system logs", paths.archon_data.join("logs")),
        probe(
            "web action audit",
            paths.archon_home.join("web/actions.audit.jsonl"),
        ),
        probe(
            "reasoning quality events",
            paths.reasoning_quality_root.join("events.jsonl"),
        ),
        probe(
            "world advisor events",
            paths
                .world_model_root
                .join("ledgers/world-advisor-events.jsonl"),
        ),
    ]
}

fn store_health(paths: &WebRuntimePaths, dist: &PathProbe) -> Vec<MetricStoreHealth> {
    [
        probe("session database", paths.session_db.clone()),
        probe("session activity", paths.session_activity_root.clone()),
        probe("memory database", paths.memory_db.clone()),
        probe("world model", paths.world_model_root.clone()),
        probe("reasoning quality", paths.reasoning_quality_root.clone()),
        dist.clone(),
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

fn ledger_values(paths: &WebRuntimePaths) -> Vec<MetricValue> {
    vec![
        ledger(
            "web action audit ledger",
            paths.archon_home.join("web/actions.audit.jsonl"),
        ),
        ledger(
            "reasoning-quality event ledger",
            paths.reasoning_quality_root.join("events.jsonl"),
        ),
        ledger(
            "world advisor event ledger",
            paths
                .world_model_root
                .join("ledgers/world-advisor-events.jsonl"),
        ),
    ]
}

fn ledger(label: &str, path: PathBuf) -> MetricValue {
    let exists = path.exists();
    let lines = count_lines(&path);
    metric(
        label,
        lines.to_string(),
        "rows",
        if exists && lines > 0 {
            "active"
        } else if exists || path.parent().is_some_and(Path::exists) {
            "quiet"
        } else {
            "missing"
        },
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

fn provider_runtime_records(
    cwd: &Path,
    paths: &WebRuntimePaths,
) -> Vec<ProviderRuntimeEventRecord> {
    let mut seen = BTreeSet::new();
    let mut records = Vec::new();
    for path in learning_db_candidates(cwd, paths) {
        if !path.exists() {
            continue;
        }
        let path_str = path.to_string_lossy().to_string();
        let Ok(db) = cozo::DbInstance::new("sqlite", &path_str, "") else {
            continue;
        };
        let Ok(events) = archon_learning::runtime_events::list_provider_runtime_events(&db, None)
        else {
            continue;
        };
        for event in events {
            if seen.insert(event.event_id.clone()) {
                records.push(event);
            }
        }
    }
    records.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    records
}

fn learning_db_candidates(cwd: &Path, paths: &WebRuntimePaths) -> Vec<PathBuf> {
    let mut candidates = vec![
        paths.archon_data.join("learning.db"),
        cwd.join(".archon/learning.db"),
    ];
    if let Some(parent) = cwd.parent() {
        candidates.push(parent.join("learning.db"));
    }
    if let Some(parent) = paths.session_db.parent() {
        candidates.push(parent.join("learning.db"));
    }
    if let Some(parent) = paths.session_db.parent().and_then(|dir| dir.parent()) {
        candidates.push(parent.join("learning.db"));
    }
    candidates.push(paths.archon_home.join("learning.db"));
    candidates.sort();
    candidates.dedup();
    candidates
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

fn embedded_asset_probe() -> PathProbe {
    let asset_paths = assets::list_assets();
    let bytes = asset_paths
        .iter()
        .filter_map(|path| assets::get_asset(path))
        .map(|asset| asset.data.len() as u64)
        .sum();
    PathProbe {
        label: "web assets".into(),
        path: "embedded web/dist assets".into(),
        exists: !asset_paths.is_empty(),
        files: asset_paths.len() as u64,
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
mod tests;
