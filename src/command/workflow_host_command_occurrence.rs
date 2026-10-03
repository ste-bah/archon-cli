//! A content identity names an artifact; an occurrence names its evaluation.
//! Repeated evaluations keep separate slots, so a pause can replay each answer
//! in order and a fresh identical candidate never consumes a historical answer.
use std::collections::BTreeMap;
use std::sync::Mutex;

pub(crate) const OCCURRENCE_KEY: &str = "host_command_occurrence";

#[derive(Default)]
pub(crate) struct HostCommandOccurrences(Mutex<BTreeMap<String, u64>>);

impl HostCommandOccurrences {
    /// The ordinal is local to one script execution and one content identity.
    /// A replay follows the same requests, including identical candidates, so
    /// its ordinals match history. Requests beyond history get new ordinals.
    pub(crate) fn next(&self, identity: String) -> (String, u64) {
        let mut counts = self.0.lock().unwrap_or_else(|error| error.into_inner());
        let occurrence = counts.entry(identity.clone()).or_default();
        *occurrence += 1;
        (occurrence_identity(&identity, *occurrence), *occurrence)
    }
}

pub(crate) fn occurrence_identity(identity: &str, occurrence: u64) -> String {
    if occurrence <= 1 {
        identity.to_string()
    } else {
        format!("{identity}:occurrence:{occurrence}")
    }
}

/// Verify the host-owned occurrence against the executor's content identity.
/// Legacy records without an ordinal retain the original identity check.
pub(crate) fn record_identity_matches(
    record: &archon_workflow::WorkflowV2CallRecord,
    identity: &str,
) -> bool {
    let occurrence = record.call.options.extra.get(OCCURRENCE_KEY);
    match occurrence {
        None => record.call.id == identity,
        Some(value) => value.as_u64().is_some_and(|ordinal| {
            ordinal > 0 && record.call.id == occurrence_identity(identity, ordinal)
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recorded_occurrence_is_checked_against_the_executor_identity() {
        let mut record = archon_workflow::WorkflowV2CallRecord::new(
            "run",
            archon_workflow::WorkflowV2HostCall {
                id: "artifact:occurrence:2".into(),
                method: archon_workflow::WorkflowV2HostMethod::HostCommand,
                write_mode: None,
                options: Default::default(),
            },
            1,
            "input".into(),
            Default::default(),
            Vec::new(),
        );
        record
            .call
            .options
            .extra
            .insert(OCCURRENCE_KEY.into(), serde_json::json!(2));
        assert!(record_identity_matches(&record, "artifact"));
        assert!(!record_identity_matches(&record, "different artifact"));
        for invalid in [
            serde_json::json!(0),
            serde_json::json!(3),
            serde_json::json!("2"),
        ] {
            record
                .call
                .options
                .extra
                .insert(OCCURRENCE_KEY.into(), invalid);
            assert!(!record_identity_matches(&record, "artifact"));
        }
        record.call.options.extra.clear();
        assert!(!record_identity_matches(&record, "artifact"));
        record.call.id = "artifact".into();
        assert!(record_identity_matches(&record, "artifact"));
    }

    #[test]
    fn identical_content_has_distinct_occurrences_and_replay_restarts_the_sequence() {
        let first = HostCommandOccurrences::default();
        let replay = HostCommandOccurrences::default();
        let expected: Vec<_> = (1..=100).map(|_| first.next("artifact".into())).collect();
        for answer in &expected {
            assert_eq!(&replay.next("artifact".into()), answer);
        }
        let fresh = replay.next("artifact".into());
        assert!(!expected.contains(&fresh));
        assert_eq!(first.next("different artifact".into()).1, 1);
    }
}
