use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::Utc;
use cozo::DbInstance;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::cli_args::{SelfAction, SelfPlansAction, SelfTrustAction};

#[derive(Clone, Debug, Serialize, Deserialize)]
struct TolerantActivityRead {
    events: Vec<archon_observability::AgentActivityEvent>,
    skipped_lines: Vec<SkippedLine>,
    source_hash: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
struct SkippedLine {
    line: usize,
    error: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
struct RetrospectiveCandidate {
    category: String,
    domain: String,
    content: String,
    confidence: f32,
    evidence_event_ids: Vec<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
struct RetrospectiveReport {
    session_id: String,
    source_activity_log: String,
    source_activity_hash: String,
    extracted_at: String,
    accepted: Vec<AcceptedLearning>,
    skipped: Vec<SkippedLearning>,
    skipped_lines: Vec<SkippedLine>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
struct AcceptedLearning {
    candidate: RetrospectiveCandidate,
    memory_id: Option<String>,
    learning_event_id: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
struct SkippedLearning {
    candidate: RetrospectiveCandidate,
    reason: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct SelfTrustFile {
    records: BTreeMap<String, SelfTrustRecord>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct SelfTrustRecord {
    domain: String,
    positive_evidence_count: u32,
    negative_evidence_count: u32,
    smoothed_trust_score: f32,
    last_update_source: Option<String>,
    correction_classes: BTreeMap<String, u32>,
    confidence_notes: Vec<String>,
}

pub async fn handle_self_command(action: SelfAction) -> Result<()> {
    match action {
        SelfAction::Retrospective { session_id } => retrospective(&session_id),
        SelfAction::Trust {
            action: SelfTrustAction::Status,
        } => trust_status(),
        SelfAction::Plans {
            action: SelfPlansAction::Inspect { session_id },
        } => inspect_plans(&session_id),
    }
}

fn retrospective(session_id: &str) -> Result<()> {
    let activity_path = activity_path(session_id)?;
    let read = read_activity_tolerant(&activity_path)
        .with_context(|| format!("read {}", activity_path.display()))?;
    let candidates = extract_candidates(&read.events);
    let base = calibration_root()?;
    let report_path = base
        .join("retrospectives")
        .join(format!("{session_id}.json"));
    let previous = read_previous_report(&report_path)?;
    let mut accepted = Vec::new();
    let mut skipped = Vec::new();
    let memory = archon_memory::graph::MemoryGraph::open_default().ok();
    let learning_db = open_learning_db().ok();

    for candidate in candidates.into_iter().take(3) {
        if previous.contains(&candidate.content) {
            skipped.push(SkippedLearning {
                candidate,
                reason: "duplicate retrospective candidate".into(),
            });
            continue;
        }
        if let Some(graph) = &memory
            && graph
                .recall_memories(&candidate.content, 5)
                .map(|memories| memories.iter().any(|m| m.content == candidate.content))
                .unwrap_or(false)
        {
            skipped.push(SkippedLearning {
                candidate,
                reason: "memory already exists".into(),
            });
            continue;
        }

        let memory_id = memory.as_ref().and_then(|graph| {
            graph
                .store_memory(
                    &candidate.content,
                    &format!("Retrospective: {}", candidate.category),
                    archon_memory::MemoryType::Pattern,
                    f64::from(candidate.confidence),
                    &[
                        "retrospective".to_string(),
                        candidate.category.clone(),
                        session_id.to_string(),
                    ],
                    "session-retrospective",
                    &std::env::current_dir()
                        .unwrap_or_else(|_| PathBuf::from("."))
                        .display()
                        .to_string(),
                )
                .ok()
        });

        let learning_event_id = learning_db.as_ref().and_then(|db| {
            archon_learning::events::record_event(
                db,
                "default",
                event_type_for(&candidate.category),
                &format!("session:{session_id}"),
                memory_id.as_deref(),
                serde_json::json!({
                    "category": candidate.category.clone(),
                    "domain": candidate.domain.clone(),
                    "content": candidate.content.clone(),
                    "evidence_event_ids": candidate.evidence_event_ids.clone(),
                }),
                candidate.confidence,
                &format!("activity:{}", read.source_hash),
            )
            .ok()
            .map(|event| event.event_id)
        });

        update_trust(
            &candidate.domain,
            false,
            &candidate.category,
            &format!("retrospective:{session_id}"),
        )?;
        accepted.push(AcceptedLearning {
            candidate,
            memory_id,
            learning_event_id,
        });
    }

    let report = RetrospectiveReport {
        session_id: session_id.to_string(),
        source_activity_log: activity_path.display().to_string(),
        source_activity_hash: read.source_hash,
        extracted_at: Utc::now().to_rfc3339(),
        accepted,
        skipped,
        skipped_lines: read.skipped_lines,
    };
    write_json(&report_path, &report)?;
    println!("Retrospective: {}", report.session_id);
    println!("Accepted learnings: {}", report.accepted.len());
    println!("Skipped candidates: {}", report.skipped.len());
    println!("Skipped malformed lines: {}", report.skipped_lines.len());
    println!("Report: {}", report_path.display());
    Ok(())
}

fn trust_status() -> Result<()> {
    let trust = load_trust()?;
    if trust.records.is_empty() {
        println!("No self-trust records found yet.");
        return Ok(());
    }
    println!(
        "{:<26} {:>5} {:>5} {:>7}  LAST SOURCE",
        "DOMAIN", "OK", "MISS", "TRUST"
    );
    println!("{}", "-".repeat(72));
    for record in trust.records.values() {
        println!(
            "{:<26} {:>5} {:>5} {:>6.2}  {}",
            record.domain,
            record.positive_evidence_count,
            record.negative_evidence_count,
            record.smoothed_trust_score,
            record.last_update_source.as_deref().unwrap_or("-"),
        );
    }
    Ok(())
}

fn inspect_plans(session_id: &str) -> Result<()> {
    let store = archon_session::storage::SessionStore::open_default()
        .map_err(|e| anyhow::anyhow!("open session store: {e}"))?;
    let plans = archon_session::plan::PlanStore::new(store.db())?;
    let Some(plan) = plans.load_latest_plan(session_id)? else {
        println!("No plan artifacts found for session {session_id}.");
        return Ok(());
    };

    let mut completed = 0usize;
    let mut skipped = 0usize;
    let mut blocked = 0usize;
    let mut changed = 0usize;
    for step in &plan.steps {
        match step.status {
            archon_session::plan::PlanStepStatus::Complete => completed += 1,
            archon_session::plan::PlanStepStatus::Skipped => skipped += 1,
            archon_session::plan::PlanStepStatus::Pending
            | archon_session::plan::PlanStepStatus::InProgress => blocked += 1,
        }
        if step.description.to_lowercase().contains("changed") {
            changed += 1;
        }
    }
    let total = plan.steps.len();
    let planning_accuracy = if total == 0 {
        0.0
    } else {
        (completed + skipped) as f32 / total as f32
    };
    let report = serde_json::json!({
        "session_id": session_id,
        "plan_id": plan.id.clone(),
        "title": plan.title.clone(),
        "compared_at": Utc::now().to_rfc3339(),
        "total_steps": total,
        "completed": completed,
        "skipped": skipped,
        "blocked": blocked,
        "changed": changed,
        "unplanned": 0,
        "planning_accuracy": planning_accuracy,
    });
    let report_path = calibration_root()?
        .join("plans")
        .join(format!("{session_id}.json"));
    write_json(&report_path, &report)?;
    update_trust(
        "architecture-advice",
        planning_accuracy >= 0.8,
        "planning_miss",
        session_id,
    )?;

    println!("Plan: {}", plan.title);
    println!("Steps: {total}");
    println!("Completed: {completed}");
    println!("Skipped: {skipped}");
    println!("Blocked: {blocked}");
    println!("Planning accuracy: {:.2}", planning_accuracy);
    println!("Report: {}", report_path.display());
    Ok(())
}

fn extract_candidates(
    events: &[archon_observability::AgentActivityEvent],
) -> Vec<RetrospectiveCandidate> {
    let mut out = Vec::new();
    for event in events {
        let message = event.message.to_lowercase();
        let (category, domain, content, confidence) = if message.contains("there is no")
            || message.contains("no such file")
            || message.contains("not found")
            || message.contains("wrong source")
            || message.contains("wrong repo")
        {
            (
                "source_tree_mistake",
                "rust-codebase-analysis",
                "Verify the actual source tree before making architecture or path claims.",
                0.90,
            )
        } else if matches!(
            event.kind,
            archon_observability::AgentActivityKind::ToolFailed
                | archon_observability::AgentActivityKind::AgentFailed
        ) {
            (
                "bug_pattern",
                "provider-debugging",
                "Treat failed tools and failed agents as learning evidence, not disposable noise.",
                0.75,
            )
        } else if message.contains("test failed") || message.contains("verify failed") {
            (
                "verification_habit",
                "rust-codebase-analysis",
                "When verification fails, report the concrete failing command and evidence.",
                0.80,
            )
        } else {
            continue;
        };
        out.push(RetrospectiveCandidate {
            category: category.into(),
            domain: domain.into(),
            content: content.into(),
            confidence,
            evidence_event_ids: vec![event.event_id.clone()],
        });
    }
    dedupe_candidates(out)
}

fn dedupe_candidates(candidates: Vec<RetrospectiveCandidate>) -> Vec<RetrospectiveCandidate> {
    let mut seen = BTreeSet::new();
    candidates
        .into_iter()
        .filter(|candidate| seen.insert(candidate.content.clone()))
        .collect()
}

fn read_activity_tolerant(path: &Path) -> Result<TolerantActivityRead> {
    let raw = fs::read(path)?;
    let source_hash = hex::encode(Sha256::digest(&raw));
    let reader = BufReader::new(raw.as_slice());
    let mut events = Vec::new();
    let mut skipped_lines = Vec::new();
    for (idx, line) in reader.lines().enumerate() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str(&line) {
            Ok(event) => events.push(event),
            Err(error) => skipped_lines.push(SkippedLine {
                line: idx + 1,
                error: error.to_string(),
            }),
        }
    }
    Ok(TolerantActivityRead {
        events,
        skipped_lines,
        source_hash,
    })
}

fn update_trust(domain: &str, positive: bool, class: &str, source: &str) -> Result<()> {
    let mut file = load_trust()?;
    let record = file
        .records
        .entry(domain.to_string())
        .or_insert_with(|| SelfTrustRecord::new(domain));
    if positive {
        record.positive_evidence_count += 1;
    } else {
        record.negative_evidence_count += 1;
        *record
            .correction_classes
            .entry(class.to_string())
            .or_insert(0) += 1;
    }
    record.smoothed_trust_score = (record.positive_evidence_count as f32 + 1.0)
        / (record.positive_evidence_count as f32 + record.negative_evidence_count as f32 + 2.0);
    record.last_update_source = Some(source.to_string());
    if record.confidence_notes.len() < 8 {
        record.confidence_notes.push(format!("{source}: {class}"));
    }
    write_json(&trust_path()?, &file)
}

impl SelfTrustRecord {
    fn new(domain: &str) -> Self {
        Self {
            domain: domain.to_string(),
            positive_evidence_count: 0,
            negative_evidence_count: 0,
            smoothed_trust_score: 0.5,
            last_update_source: None,
            correction_classes: BTreeMap::new(),
            confidence_notes: Vec::new(),
        }
    }
}

fn load_trust() -> Result<SelfTrustFile> {
    let path = trust_path()?;
    let mut trust = if !path.exists() {
        SelfTrustFile {
            records: BTreeMap::new(),
        }
    } else {
        serde_json::from_str(&fs::read_to_string(path)?)?
    };
    for domain in [
        "rust-codebase-analysis",
        "cli-behavior",
        "architecture-advice",
        "documentation-claims",
        "provider-debugging",
    ] {
        trust
            .records
            .entry(domain.to_string())
            .or_insert_with(|| SelfTrustRecord::new(domain));
    }
    Ok(trust)
}

fn read_previous_report(path: &Path) -> Result<BTreeSet<String>> {
    if !path.exists() {
        return Ok(BTreeSet::new());
    }
    let report: RetrospectiveReport = serde_json::from_str(&fs::read_to_string(path)?)?;
    Ok(report
        .accepted
        .into_iter()
        .map(|accepted| accepted.candidate.content)
        .chain(
            report
                .skipped
                .into_iter()
                .map(|skipped| skipped.candidate.content),
        )
        .collect())
}

fn event_type_for(category: &str) -> archon_learning::models::LearningEventType {
    match category {
        "source_tree_mistake" => archon_learning::models::LearningEventType::SourceContradicted,
        "verification_habit" => archon_learning::models::LearningEventType::GatePassed,
        _ => archon_learning::models::LearningEventType::GateFailed,
    }
}

fn open_learning_db() -> Result<DbInstance> {
    let base = archon_session::storage::default_db_path();
    let parent = base
        .parent()
        .ok_or_else(|| anyhow::anyhow!("cannot determine learning data directory"))?;
    let path = parent.join("learning.db");
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let path_str = path.to_string_lossy().to_string();
    let db = DbInstance::new("sqlite", &path_str, "")
        .map_err(|e| anyhow::anyhow!("open learning db: {e}"))?;
    archon_learning::schema::ensure_learning_schema(&db)?;
    Ok(db)
}

fn activity_path(session_id: &str) -> Result<PathBuf> {
    let base = dirs::home_dir()
        .ok_or_else(|| anyhow::anyhow!("cannot determine home directory"))?
        .join(".archon")
        .join("sessions");
    Ok(archon_observability::activity_jsonl_path(base, session_id))
}

fn trust_path() -> Result<PathBuf> {
    Ok(calibration_root()?.join("trust").join("self-trust.json"))
}

fn calibration_root() -> Result<PathBuf> {
    Ok(std::env::current_dir()?
        .join(".archon")
        .join("self-calibration"))
}

fn write_json(path: &Path, value: &impl Serialize) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, serde_json::to_string_pretty(value)?)?;
    Ok(())
}
