use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::canonical::hash_hex;
use crate::types::{ReasoningEventKind, ReasoningQualityEvent, ReasoningSubject};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepeatedReasoningPattern {
    pub pattern_id: String,
    pub event_kind: ReasoningEventKind,
    pub subject: ReasoningSubject,
    pub entity_key: String,
    pub event_count: usize,
    pub distinct_sessions: usize,
    pub shadow: bool,
}

pub fn detect_repeated_patterns(
    events: &[ReasoningQualityEvent],
    window_days: u32,
    min_events: usize,
    min_distinct_sessions: usize,
    shadow_active: bool,
) -> Vec<RepeatedReasoningPattern> {
    let cutoff = chrono::Utc::now() - chrono::Duration::days(window_days as i64);
    let mut clusters: BTreeMap<PatternKey, Cluster> = BTreeMap::new();

    for event in events {
        if event.created_at < cutoff || !is_pattern_candidate(event.event_kind) {
            continue;
        }
        let key = PatternKey {
            event_kind: event.event_kind,
            subject: event.subject,
            entity_key: normalize_entity_key(&event.entity_key),
        };
        let cluster = clusters.entry(key).or_default();
        cluster.event_count += 1;
        cluster.sessions.insert(event.session_id.clone());
    }

    let mut patterns = Vec::new();
    for (key, cluster) in clusters {
        if cluster.event_count < min_events || cluster.sessions.len() < min_distinct_sessions {
            continue;
        }
        patterns.push(RepeatedReasoningPattern {
            pattern_id: pattern_id(&key),
            event_kind: if shadow_active {
                ReasoningEventKind::ShadowRepeatedPattern
            } else {
                key.event_kind
            },
            subject: key.subject,
            entity_key: key.entity_key,
            event_count: cluster.event_count,
            distinct_sessions: cluster.sessions.len(),
            shadow: shadow_active,
        });
    }
    patterns.sort_by(|left, right| {
        right
            .event_count
            .cmp(&left.event_count)
            .then_with(|| right.distinct_sessions.cmp(&left.distinct_sessions))
            .then_with(|| left.pattern_id.cmp(&right.pattern_id))
    });
    patterns
}

fn is_pattern_candidate(kind: ReasoningEventKind) -> bool {
    matches!(
        kind,
        ReasoningEventKind::ClaimBeforeSourceRead
            | ReasoningEventKind::UnsupportedClaim
            | ReasoningEventKind::ClaimCorrectedByUser
            | ReasoningEventKind::ClaimContradictedBySource
            | ReasoningEventKind::CompletionClaimWithoutEvidence
            | ReasoningEventKind::TestStatusClaimWithoutCommand
            | ReasoningEventKind::VerificationNeeded
    )
}

fn normalize_entity_key(entity_key: &str) -> String {
    let lower = entity_key.trim().to_ascii_lowercase();
    if lower.is_empty() {
        return "general".to_string();
    }
    lower
        .split('/')
        .map(|part| if part.len() > 40 { "<long>" } else { part })
        .collect::<Vec<_>>()
        .join("/")
}

fn pattern_id(key: &PatternKey) -> String {
    format!(
        "rqpat_{}",
        &hash_hex(&format!(
            "{:?}:{:?}:{}",
            key.event_kind, key.subject, key.entity_key
        ))[..16]
    )
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct PatternKey {
    event_kind: ReasoningEventKind,
    subject: ReasoningSubject,
    entity_key: String,
}

#[derive(Default)]
struct Cluster {
    event_count: usize,
    sessions: BTreeSet<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeated_pattern_requires_three_sessions() {
        let events = ["s1", "s2", "s3"]
            .into_iter()
            .map(|session| ReasoningQualityEvent {
                session_id: session.to_string(),
                event_kind: ReasoningEventKind::ClaimBeforeSourceRead,
                subject: ReasoningSubject::Codebase,
                entity_key: "src/lib.rs".into(),
                created_at: chrono::Utc::now(),
                ..ReasoningQualityEvent::default()
            })
            .collect::<Vec<_>>();
        let patterns = detect_repeated_patterns(&events, 30, 3, 3, true);
        assert_eq!(patterns.len(), 1);
        assert_eq!(
            patterns[0].event_kind,
            ReasoningEventKind::ShadowRepeatedPattern
        );
        assert!(patterns[0].shadow);
    }
}
