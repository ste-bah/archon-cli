//! The runtime transitions of a fixed decomposition (Issue 358).
//!
//! A resume on a build whose script, catalog, template or revision differs
//! from the last one records one transition. The record is one atomically
//! replaced file, apart from the append-only event log, so a torn event line
//! (an interrupted append of any event) never stands between a run and its
//! resume. The event and the decomposition log line are its visible copies,
//! each written at most once for a transition, so a crash between the writes
//! is completed by the next resume and never repeated.
//!
//! Every field added here needs a schema bump, as for the decomposition state
//! (`FIXED_DECOMPOSITION_STATE_SCHEMA_VERSION`).

use archon_workflow::{FixedRunIdentityV1, WorkflowStore};

pub(crate) const TRANSITIONS_PATH: &str = "decomposition/runtime-transitions.json";
pub(crate) const TRANSITIONS_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RuntimeTransitions {
    pub(crate) schema_version: u32,
    pub(crate) transitions: Vec<RuntimeTransition>,
}

impl Default for RuntimeTransitions {
    fn default() -> Self {
        Self {
            schema_version: TRANSITIONS_SCHEMA_VERSION,
            transitions: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RuntimeTransition {
    /// `decomposition_runtime_upgrade`, or `binary_revision_drift` when only
    /// the binary revision changed.
    pub(crate) label: String,
    pub(crate) recorded_at: String,
    pub(crate) old: FixedRunIdentityV1,
    /// What this build runs. `catalog_digest` is the digest of the
    /// capabilities this build executes, under the launch revision that names
    /// the run's call keys.
    pub(crate) new: FixedRunIdentityV1,
    /// The seq of the visible event, once it is written.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) event_id: Option<u64>,
    /// When an executor on this runtime first started, after every resume
    /// check passed. Absent, the runtime was admitted and never executed: a
    /// later check (provider, read scope, task root) refused.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) started_at: Option<String>,
}

/// Same script, catalog, template and binary revision. Root identity is
/// verified separately; it never changes across a resume.
pub(crate) fn same_runtime(a: &FixedRunIdentityV1, b: &FixedRunIdentityV1) -> bool {
    !harness_changed(a, b) && a.starting_binary_revision == b.starting_binary_revision
}

pub(crate) fn harness_changed(a: &FixedRunIdentityV1, b: &FixedRunIdentityV1) -> bool {
    a.template_version != b.template_version
        || a.script_digest != b.script_digest
        || a.catalog_digest != b.catalog_digest
}

impl RuntimeTransition {
    pub(crate) fn new(old: FixedRunIdentityV1, new: FixedRunIdentityV1) -> Self {
        let label = if harness_changed(&old, &new) {
            "decomposition_runtime_upgrade"
        } else {
            "binary_revision_drift"
        };
        Self {
            label: label.to_string(),
            recorded_at: chrono::Utc::now().to_rfc3339(),
            old,
            new,
            event_id: None,
            started_at: None,
        }
    }

    /// The fields of the log line after `event_id`; the line's key is its
    /// first two fields, `event_id=<seq> transition=<label>`.
    pub(crate) fn log_line(&self, seq: u64) -> String {
        let field = crate::command::workflow_decompose_events::log_field;
        let (old, new) = (&self.old, &self.new);
        format!(
            "event_id={seq} transition={} persisted={} current={} template={}->{} script={}->{} catalog={}->{}",
            self.label,
            field(&old.starting_binary_revision),
            field(&new.starting_binary_revision),
            field(&old.template_version),
            field(&new.template_version),
            field(&old.script_digest),
            field(&new.script_digest),
            field(&old.catalog_digest),
            field(&new.catalog_digest),
        )
    }

    /// What a resume prints when it records this transition.
    pub(crate) fn summary(&self) -> Vec<String> {
        let (old, new) = (&self.old, &self.new);
        let mut lines = Vec::new();
        if old.starting_binary_revision != new.starting_binary_revision {
            lines.push(format!(
                "Binary revision drift: persisted={} current={}\n",
                old.starting_binary_revision, new.starting_binary_revision
            ));
        }
        if harness_changed(old, new) {
            lines.push(format!(
                "Decomposition runtime upgrade: template {}->{} script {}->{} catalog {}->{}; completed calls whose own inputs still match are reused\n",
                old.template_version,
                new.template_version,
                old.script_digest,
                new.script_digest,
                old.catalog_digest,
                new.catalog_digest,
            ));
        }
        lines
    }
}

/// The status lines for the run's transitions: none before the first one.
/// Status reports what it reads; a resume refuses an unreadable record.
pub(crate) fn status_lines(store: &WorkflowStore, run_id: &str) -> String {
    let path = store.run_dir(run_id).join(TRANSITIONS_PATH);
    let raw = match std::fs::read(&path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return String::new(),
        Err(error) => return format!("runtime_transitions: unreadable ({error})\n"),
    };
    match serde_json::from_slice::<RuntimeTransitions>(&raw) {
        Ok(record) => match record.transitions.last() {
            Some(last) => format!(
                "runtime_transitions: {}\nlast_runtime: template_version={} binary_revision={} script_digest={} catalog_digest={} started={}\n",
                record.transitions.len(),
                last.new.template_version,
                last.new.starting_binary_revision,
                last.new.script_digest,
                last.new.catalog_digest,
                last.started_at
                    .as_deref()
                    .unwrap_or("never (admitted, not executed)"),
            ),
            None => String::new(),
        },
        Err(error) => format!("runtime_transitions: unreadable ({error})\n"),
    }
}
