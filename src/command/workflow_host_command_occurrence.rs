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
    pub(crate) fn next(&self, identity: &str) -> u64 {
        let mut counts = self.0.lock().unwrap_or_else(|error| error.into_inner());
        let occurrence = counts.entry(identity.to_string()).or_default();
        *occurrence += 1;
        *occurrence
    }
}

/// Names one evaluation of a host command. A first occurrence keeps the
/// content identity, the call options and the input exactly as a binary
/// without occurrences wrote them, so the input hash is unchanged and a run in
/// flight across a deploy reuses its records. Only a repeat carries the
/// ordinal, in its id, its options and its input.
pub(crate) fn stamp_occurrence(
    call: &mut archon_workflow::WorkflowV2HostCall,
    input: &mut serde_json::Value,
    identity: &str,
    occurrence: u64,
) {
    if occurrence > 1 {
        call.options
            .extra
            .insert(OCCURRENCE_KEY.into(), serde_json::json!(occurrence));
        input[OCCURRENCE_KEY] = serde_json::json!(occurrence);
    }
    let id = occurrence_identity(identity, occurrence);
    call.id = id.clone();
    input["call_id"] = serde_json::Value::String(id);
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
        let expected: Vec<_> = (1..=100).map(|_| first.next("artifact")).collect();
        for answer in &expected {
            assert_eq!(&replay.next("artifact"), answer);
        }
        let fresh = replay.next("artifact");
        assert!(!expected.contains(&fresh));
        assert_eq!(first.next("different artifact"), 1);
    }

    /// Records written before occurrences existed carry no ordinal. A first
    /// occurrence must produce the same call, input and input hash, so those
    /// records still match; a repeat must not.
    #[test]
    fn a_first_occurrence_matches_a_record_written_without_occurrences() {
        let call = archon_workflow::WorkflowV2HostCall {
            id: "script-call".into(),
            method: archon_workflow::WorkflowV2HostMethod::HostCommand,
            write_mode: None,
            options: Default::default(),
        };
        let input = serde_json::json!({"objective": "o", "call_id": "script-call", "options": {"stdin": "x"}});
        let (mut legacy_call, mut legacy_input) = (call.clone(), input.clone());
        legacy_call.id = "artifact".into();
        legacy_input["call_id"] = serde_json::json!("artifact");
        let hash = |value: &serde_json::Value| {
            archon_workflow::v2::source_graph::input_hash_with_source_fingerprint(value, None)
        };

        let (mut first_call, mut first_input) = (call.clone(), input.clone());
        stamp_occurrence(&mut first_call, &mut first_input, "artifact", 1);
        assert_eq!(first_call, legacy_call);
        assert_eq!(first_input, legacy_input);
        assert_eq!(hash(&first_input), hash(&legacy_input));
        let legacy_record = archon_workflow::WorkflowV2CallRecord::new(
            "run",
            legacy_call,
            1,
            hash(&legacy_input),
            Default::default(),
            Vec::new(),
        );
        assert!(record_identity_matches(&legacy_record, "artifact"));

        let (mut repeat_call, mut repeat_input) = (call, input);
        stamp_occurrence(&mut repeat_call, &mut repeat_input, "artifact", 2);
        assert_eq!(repeat_call.id, "artifact:occurrence:2");
        assert_eq!(repeat_call.options.extra[OCCURRENCE_KEY], 2);
        assert_ne!(hash(&repeat_input), hash(&legacy_input));
    }
}
