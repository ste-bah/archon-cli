//! Runtime bridges for reasoning-quality events.

use std::fs::{self, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use anyhow::Result;
use serde::{Deserialize, Serialize};

pub(crate) fn bridge_reasoning_events(
    events: &[archon_reasoning_quality::ReasoningQualityEvent],
    learning_db: Option<&cozo::DbInstance>,
    reasoning_root: &Path,
    world_root: Option<&Path>,
    feed_world_model: bool,
    update_self_trust: bool,
) {
    for event in events {
        if let Some(db) = learning_db
            && let Err(error) = bridge_learning_event(db, event)
        {
            append_dead_letter(reasoning_root, "learning_event", event, &error.to_string());
        } else if learning_db.is_some() {
            let _ = append_bridge_offset(reasoning_root, "learning_event", event);
        }
        if feed_world_model
            && let Some(root) = world_root
            && let Err(error) = bridge_world_model(root, event)
        {
            append_dead_letter(reasoning_root, "world_model", event, &error.to_string());
        } else if feed_world_model && world_root.is_some() {
            let _ = append_bridge_offset(reasoning_root, "world_model", event);
        }
        if update_self_trust && event.shadow {
            if let Err(error) = append_shadow_delta(reasoning_root, event) {
                append_dead_letter(reasoning_root, "self_trust", event, &error.to_string());
            } else {
                let _ = append_bridge_offset(reasoning_root, "self_trust", event);
            }
        } else if update_self_trust && let Err(error) = apply_self_trust_update(event) {
            append_dead_letter(reasoning_root, "self_trust", event, &error.to_string());
        } else if update_self_trust {
            let _ = append_bridge_offset(reasoning_root, "self_trust", event);
        }
    }
}

fn bridge_learning_event(
    db: &cozo::DbInstance,
    event: &archon_reasoning_quality::ReasoningQualityEvent,
) -> Result<()> {
    archon_learning::events::record_event(
        db,
        "default",
        archon_learning::models::LearningEventType::ReasoningQuality,
        &format!("reasoning:{}", event.event_id),
        None,
        serde_json::json!({
            "source_system": "reasoning_quality",
            "event_id": event.event_id,
            "event_kind": event.event_kind,
            "session_id": event.session_id,
            "turn_number": event.turn_number,
            "claim_id": event.claim_id,
            "subject": event.subject,
            "verification_state": event.verification_state,
            "severity_effective": event.severity_effective,
            "shadow": event.shadow,
        }),
        event.severity_effective.clamp(0.0, 1.0),
        &event.claim_id,
    )
    .map(|_| ())
    .map_err(|e| anyhow::anyhow!("reasoning-quality LearningEvent bridge failed: {e}"))
}

fn bridge_world_model(
    root: &Path,
    event: &archon_reasoning_quality::ReasoningQualityEvent,
) -> Result<()> {
    let store = archon_world_model::storage::WorldModelStore::open(root)?;
    if reasoning_quality_cold_start_cap_reached(&store)? {
        return Ok(());
    }
    let mut row = archon_world_model::WorldTraceRow::new(
        &event.session_id,
        archon_world_model::schema::WorldActionKind::Verification,
    )
    .with_row_id(format!("world-row-rq-{}", event.event_id));
    row.source = archon_world_model::schema::WorldTraceSource::ReasoningQuality;
    row.redacted_excerpt = event.redacted_excerpt.clone();
    row.evidence_refs = event
        .evidence_refs
        .iter()
        .map(|evidence| {
            archon_world_model::schema::EvidenceRef::new(
                "reasoning_quality",
                evidence.evidence_id.clone(),
            )
        })
        .collect();
    match event.event_kind {
        archon_reasoning_quality::ReasoningEventKind::ClaimCorrectedByUser => {
            row.labels.user_correction = true;
            row.labels.failure = true;
            row.labels.success = Some(false);
        }
        archon_reasoning_quality::ReasoningEventKind::ClaimContradictedBySource => {
            row.labels.failure = true;
            row.labels.success = Some(false);
        }
        archon_reasoning_quality::ReasoningEventKind::ClaimBeforeSourceRead
        | archon_reasoning_quality::ReasoningEventKind::UnsupportedClaim
        | archon_reasoning_quality::ReasoningEventKind::CompletionClaimWithoutEvidence
        | archon_reasoning_quality::ReasoningEventKind::TestStatusClaimWithoutCommand
        | archon_reasoning_quality::ReasoningEventKind::VerificationNeeded => {
            row.labels.verification_needed = true;
        }
        archon_reasoning_quality::ReasoningEventKind::SourceVerifiedClaim => {
            row.labels.success = Some(true);
        }
        _ => {}
    }
    store.persist_rows(&[row])?;
    Ok(())
}

fn reasoning_quality_cold_start_cap_reached(
    store: &archon_world_model::storage::WorldModelStore,
) -> Result<bool> {
    let rows = store.load_rows()?;
    if rows.len() >= 1_000 {
        return Ok(false);
    }
    let reasoning_rows = rows
        .iter()
        .filter(|row| row.source == archon_world_model::schema::WorldTraceSource::ReasoningQuality)
        .count();
    Ok(reasoning_rows >= 250)
}

fn append_shadow_delta(
    root: &Path,
    event: &archon_reasoning_quality::ReasoningQualityEvent,
) -> Result<()> {
    let path = root.join("shadow").join("self-trust-deltas.jsonl");
    append_jsonl(
        &path,
        &serde_json::json!({
            "event_id": event.event_id,
            "claim_id": event.claim_id,
            "session_id": event.session_id,
            "subject": event.subject,
            "event_kind": event.event_kind,
            "severity_effective": event.severity_effective,
            "applied": false,
            "reason": "shadow_mode",
        }),
    )
}

fn append_dead_letter(
    root: &Path,
    bridge: &str,
    event: &archon_reasoning_quality::ReasoningQualityEvent,
    error: &str,
) {
    let path = root.join("dead-letter").join("bridge-failures.jsonl");
    let _ = append_jsonl(
        &path,
        &serde_json::json!({
            "bridge": bridge,
            "event_id": event.event_id,
            "claim_id": event.claim_id,
            "session_id": event.session_id,
            "event_json": event,
            "error": error,
            "created_at": chrono::Utc::now().to_rfc3339(),
        }),
    );
}

fn append_bridge_offset(
    root: &Path,
    bridge: &str,
    event: &archon_reasoning_quality::ReasoningQualityEvent,
) -> Result<()> {
    let path = root.join("bridge-offsets").join(format!("{bridge}.jsonl"));
    append_jsonl(
        &path,
        &serde_json::json!({
            "bridge": bridge,
            "event_id": event.event_id,
            "claim_id": event.claim_id,
            "session_id": event.session_id,
            "created_at": chrono::Utc::now().to_rfc3339(),
        }),
    )
}

fn apply_self_trust_update(event: &archon_reasoning_quality::ReasoningQualityEvent) -> Result<()> {
    let base = std::env::current_dir()?;
    apply_self_trust_update_at(&base, event)
}

fn apply_self_trust_update_at(
    base: &Path,
    event: &archon_reasoning_quality::ReasoningQualityEvent,
) -> Result<()> {
    let path = self_trust_path(base);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(&path)?;
    let mut lock = fd_lock::RwLock::new(file);
    let mut guard = lock
        .try_write()
        .map_err(|e| anyhow::anyhow!("self-trust lock unavailable: {e}"))?;
    let mut content = String::new();
    guard.read_to_string(&mut content)?;
    let mut trust = if content.trim().is_empty() {
        SelfTrustFile::default()
    } else {
        serde_json::from_str(&content)?
    };
    let domain = trust_domain(event.subject);
    let record = trust
        .records
        .entry(domain.to_string())
        .or_insert_with(|| SelfTrustRecord::new(domain));
    if is_positive_trust_event(event.event_kind) {
        record.positive_evidence_count = record.positive_evidence_count.saturating_add(1);
    } else {
        record.negative_evidence_count = record.negative_evidence_count.saturating_add(1);
        *record
            .correction_classes
            .entry(format!("{:?}", event.event_kind))
            .or_insert(0) += 1;
    }
    record.smoothed_trust_score = (record.positive_evidence_count as f32 + 1.0)
        / (record.positive_evidence_count as f32 + record.negative_evidence_count as f32 + 2.0);
    record.last_update_source = Some(format!("reasoning-quality:{}", event.event_id));
    if record.confidence_notes.len() < 8 {
        record
            .confidence_notes
            .push(format!("{}: {:?}", event.claim_id, event.event_kind));
    }

    guard.set_len(0)?;
    guard.seek(SeekFrom::Start(0))?;
    serde_json::to_writer_pretty(&mut *guard, &trust)?;
    guard.write_all(b"\n")?;
    Ok(())
}

fn is_positive_trust_event(kind: archon_reasoning_quality::ReasoningEventKind) -> bool {
    matches!(
        kind,
        archon_reasoning_quality::ReasoningEventKind::SourceVerifiedClaim
    )
}

fn trust_domain(subject: archon_reasoning_quality::ReasoningSubject) -> &'static str {
    match subject {
        archon_reasoning_quality::ReasoningSubject::Codebase => "rust-codebase-analysis",
        archon_reasoning_quality::ReasoningSubject::Documentation => "documentation-claims",
        archon_reasoning_quality::ReasoningSubject::ProviderStatus => "provider-debugging",
        archon_reasoning_quality::ReasoningSubject::Configuration
        | archon_reasoning_quality::ReasoningSubject::RuntimeStatus => "cli-behavior",
        archon_reasoning_quality::ReasoningSubject::ArchitectureAdvice
        | archon_reasoning_quality::ReasoningSubject::Plan => "architecture-advice",
        _ => "cli-behavior",
    }
}

fn self_trust_path(base: &Path) -> PathBuf {
    base.join(".archon")
        .join("self-calibration")
        .join("trust")
        .join("self-trust.json")
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct SelfTrustFile {
    records: std::collections::BTreeMap<String, SelfTrustRecord>,
}

#[derive(Debug, Serialize, Deserialize)]
struct SelfTrustRecord {
    domain: String,
    positive_evidence_count: u32,
    negative_evidence_count: u32,
    smoothed_trust_score: f32,
    last_update_source: Option<String>,
    correction_classes: std::collections::BTreeMap<String, u32>,
    confidence_notes: Vec<String>,
}

impl SelfTrustRecord {
    fn new(domain: &str) -> Self {
        Self {
            domain: domain.to_string(),
            positive_evidence_count: 0,
            negative_evidence_count: 0,
            smoothed_trust_score: 0.5,
            last_update_source: None,
            correction_classes: std::collections::BTreeMap::new(),
            confidence_notes: Vec::new(),
        }
    }
}

fn append_jsonl(path: &Path, value: &serde_json::Value) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    serde_json::to_writer(&mut file, value)?;
    file.write_all(b"\n")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shadow_bridge_writes_delta_without_applying_trust() {
        let temp = tempfile::tempdir().unwrap();
        let event = archon_reasoning_quality::ReasoningQualityEvent {
            event_id: "rqevt-test".into(),
            claim_id: "rqclm-test".into(),
            session_id: "s1".into(),
            shadow: true,
            ..archon_reasoning_quality::ReasoningQualityEvent::default()
        };
        bridge_reasoning_events(&[event], None, temp.path(), None, false, true);
        let content =
            fs::read_to_string(temp.path().join("shadow").join("self-trust-deltas.jsonl")).unwrap();
        assert!(content.contains("\"applied\":false"));
    }

    #[test]
    fn active_bridge_updates_self_trust_with_file_lock() {
        let temp = tempfile::tempdir().unwrap();
        let event = archon_reasoning_quality::ReasoningQualityEvent {
            event_id: "rqevt-active".into(),
            claim_id: "rqclm-active".into(),
            session_id: "s1".into(),
            shadow: false,
            event_kind: archon_reasoning_quality::ReasoningEventKind::ClaimCorrectedByUser,
            subject: archon_reasoning_quality::ReasoningSubject::Codebase,
            ..archon_reasoning_quality::ReasoningQualityEvent::default()
        };
        apply_self_trust_update_at(temp.path(), &event).unwrap();
        let content = fs::read_to_string(
            temp.path()
                .join(".archon/self-calibration/trust/self-trust.json"),
        )
        .unwrap();
        assert!(content.contains("rust-codebase-analysis"));
        assert!(content.contains("negative_evidence_count"));
    }

    #[test]
    fn bridge_offset_is_append_only() {
        let temp = tempfile::tempdir().unwrap();
        let event = archon_reasoning_quality::ReasoningQualityEvent {
            event_id: "rqevt-offset".into(),
            claim_id: "rqclm-offset".into(),
            session_id: "s1".into(),
            ..archon_reasoning_quality::ReasoningQualityEvent::default()
        };
        append_bridge_offset(temp.path(), "learning_event", &event).unwrap();
        let content = fs::read_to_string(
            temp.path()
                .join("bridge-offsets")
                .join("learning_event.jsonl"),
        )
        .unwrap();
        assert!(content.contains("rqevt-offset"));
    }

    #[test]
    fn canonical_e2e_bridge_and_briefing_candidate() {
        let temp = tempfile::tempdir().unwrap();
        let world = tempfile::tempdir().unwrap();
        let learning_path = temp.path().join("learning.db");
        let learning_db = cozo::DbInstance::new("sqlite", &learning_path, "").unwrap();
        archon_learning::schema::ensure_learning_schema(&learning_db).unwrap();
        let event = archon_reasoning_quality::ReasoningQualityEvent {
            event_id: "rqevt-e2e".into(),
            claim_id: "rqclm-e2e".into(),
            session_id: "s-e2e".into(),
            event_kind: archon_reasoning_quality::ReasoningEventKind::ClaimCorrectedByUser,
            subject: archon_reasoning_quality::ReasoningSubject::Codebase,
            entity_key: "src/lib.rs".into(),
            canonical_text: "the module exists".into(),
            redacted_excerpt: Some("the module exists".into()),
            severity_effective: 1.0,
            shadow: true,
            ..archon_reasoning_quality::ReasoningQualityEvent::default()
        };
        let store =
            archon_reasoning_quality::store::ReasoningQualityStore::open(temp.path()).unwrap();
        store.append_events(std::slice::from_ref(&event)).unwrap();

        bridge_reasoning_events(
            std::slice::from_ref(&event),
            Some(&learning_db),
            temp.path(),
            Some(world.path()),
            true,
            true,
        );

        let learning_events =
            archon_learning::store::list_all_learning_events(&learning_db).unwrap();
        assert!(
            learning_events.iter().any(|row| row.event_type
                == archon_learning::models::LearningEventType::ReasoningQuality)
        );
        let world_rows = archon_world_model::storage::WorldModelStore::open(world.path())
            .unwrap()
            .load_rows()
            .unwrap();
        assert_eq!(world_rows.len(), 1);
        assert!(
            temp.path()
                .join("shadow")
                .join("self-trust-deltas.jsonl")
                .exists()
        );

        let briefing = crate::runtime::proactive_briefing::build_session_briefing(
            &archon_core::config::ArchonConfig::default(),
            &archon_policy::models::EffectivePolicy::default(),
            Some(temp.path()),
            Some(&learning_db),
            Some(world.path()),
            "s-e2e",
            Some("check src/lib.rs"),
        )
        .unwrap();
        assert!(briefing.contains("Reasoning-quality warnings"));
    }
}
