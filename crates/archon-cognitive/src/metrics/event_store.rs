//! Append-only writer and reader for `cognitive_metric_events`.

use std::collections::BTreeMap;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};

use cozo::{DataValue, DbInstance, ScriptMutability};

use crate::CognitiveError;
use crate::cozo_guard::{relation_count, run_script_guarded};
use crate::metrics::codec::{int_col, json_col, opt_float_col, opt_float_value, str_col, time_col};
use crate::metrics::derive::{CognitiveMetricSnapshot, derive_snapshot};
use crate::metrics::event::CognitiveMetricEvent;
use crate::metrics::window::EvaluationWindow;
use crate::metrics::window_store::{WindowDeclaration, declare_window, list_windows, load_window};
use crate::schema::ensure_cognitive_schema;

const EVENT_COLUMNS: &str = "metric_event_id, idempotency_key, fingerprint, metric_name, metric_definition_version, evaluation_dataset_version, evaluation_window_id, event_kind, session_id, turn_number, task_class, model_id, policy_version, label_source, outcome_status, value, numerator, denominator, identities_json, evidence_refs_json, created_at";

const LEDGER_FILE: &str = "cognitive-metric-events.jsonl";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetricWriteOutcome {
    Written,
    /// Same event id, same content. The relation and the ledger are both left
    /// alone so a retried writer cannot double-count its own observation.
    DuplicateIgnored,
}

pub struct MetricEventStore<'a> {
    db: &'a DbInstance,
    ledger_path: PathBuf,
}

impl<'a> MetricEventStore<'a> {
    pub fn new(db: &'a DbInstance, ledger_dir: impl AsRef<Path>) -> Result<Self, CognitiveError> {
        ensure_cognitive_schema(db)?;
        let ledger_dir = ledger_dir.as_ref();
        std::fs::create_dir_all(ledger_dir)?;
        Ok(Self {
            db,
            ledger_path: ledger_dir.join(LEDGER_FILE),
        })
    }

    pub fn declare_window(
        &self,
        window: &EvaluationWindow,
    ) -> Result<WindowDeclaration, CognitiveError> {
        declare_window(self.db, window)
    }

    pub fn window(
        &self,
        evaluation_window_id: &str,
    ) -> Result<Option<EvaluationWindow>, CognitiveError> {
        load_window(self.db, evaluation_window_id)
    }

    pub fn windows(&self) -> Result<Vec<EvaluationWindow>, CognitiveError> {
        list_windows(self.db)
    }

    /// Most recently started window, which is the one the read-only surfaces
    /// report when the caller does not name one.
    pub fn latest_window(&self) -> Result<Option<EvaluationWindow>, CognitiveError> {
        Ok(self.windows()?.pop())
    }

    pub fn record(
        &self,
        event: &CognitiveMetricEvent,
    ) -> Result<MetricWriteOutcome, CognitiveError> {
        event.validate()?;
        let fingerprint = event.fingerprint()?;
        if let Some(stored) = self.stored_fingerprint(&event.metric_event_id)? {
            if stored == fingerprint {
                return Ok(MetricWriteOutcome::DuplicateIgnored);
            }
            return Err(CognitiveError::Metric(format!(
                "metric event `{}` already exists with different content",
                event.metric_event_id
            )));
        }
        self.reject_idempotency_key_reuse(event)?;

        run_script_guarded(
            self.db,
            &format!(
                "?[{EVENT_COLUMNS}] <- [[$metric_event_id, $idempotency_key, $fingerprint, $metric_name, $metric_definition_version, $evaluation_dataset_version, $evaluation_window_id, $event_kind, $session_id, $turn_number, $task_class, $model_id, $policy_version, $label_source, $outcome_status, $value, $numerator, $denominator, $identities_json, $evidence_refs_json, $created_at]]
                 :put cognitive_metric_events {{ metric_event_id => idempotency_key, fingerprint, metric_name, metric_definition_version, evaluation_dataset_version, evaluation_window_id, event_kind, session_id, turn_number, task_class, model_id, policy_version, label_source, outcome_status, value, numerator, denominator, identities_json, evidence_refs_json, created_at }}"
            ),
            event_params(event, &fingerprint)?,
            ScriptMutability::Mutable,
            "put cognitive metric event",
        )?;
        self.append_ledger(event)?;
        Ok(MetricWriteOutcome::Written)
    }

    pub fn event_count(&self) -> usize {
        relation_count(self.db, "cognitive_metric_events", "metric_event_id").unwrap_or(0)
    }

    pub fn events(&self) -> Result<Vec<CognitiveMetricEvent>, CognitiveError> {
        let rows = run_script_guarded(
            self.db,
            &format!("?[{EVENT_COLUMNS}] := *cognitive_metric_events{{{EVENT_COLUMNS}}}"),
            Default::default(),
            ScriptMutability::Immutable,
            "list cognitive metric events",
        )?;
        let mut events = rows
            .rows
            .iter()
            .map(|row| row_to_event(row))
            .collect::<Result<Vec<_>, _>>()?;
        events.sort_by(|left, right| {
            left.created_at
                .cmp(&right.created_at)
                .then_with(|| left.metric_event_id.cmp(&right.metric_event_id))
        });
        Ok(events)
    }

    /// Recompute the snapshot for `window`, or for the whole history when no
    /// window has been declared yet.
    pub fn snapshot(
        &self,
        window: Option<&EvaluationWindow>,
    ) -> Result<CognitiveMetricSnapshot, CognitiveError> {
        Ok(derive_snapshot(window, &self.events()?))
    }

    pub fn latest_snapshot(&self) -> Result<CognitiveMetricSnapshot, CognitiveError> {
        let window = self.latest_window()?;
        self.snapshot(window.as_ref())
    }

    fn stored_fingerprint(&self, metric_event_id: &str) -> Result<Option<String>, CognitiveError> {
        let mut params = BTreeMap::new();
        params.insert("metric_event_id".into(), DataValue::from(metric_event_id));
        let rows = run_script_guarded(
            self.db,
            "?[fingerprint] := *cognitive_metric_events{metric_event_id: $metric_event_id, fingerprint}",
            params,
            ScriptMutability::Immutable,
            "read cognitive metric event fingerprint",
        )?;
        Ok(rows.rows.first().map(|row| str_col(row, 0)))
    }

    /// Two different event ids sharing one idempotency key is a duplicate
    /// identity, not a replay: the completion standard treats it as a
    /// conflict rather than letting both rows into the denominator.
    fn reject_idempotency_key_reuse(
        &self,
        event: &CognitiveMetricEvent,
    ) -> Result<(), CognitiveError> {
        let mut params = BTreeMap::new();
        params.insert(
            "idempotency_key".into(),
            DataValue::from(event.idempotency_key.as_str()),
        );
        let rows = run_script_guarded(
            self.db,
            "?[metric_event_id] := *cognitive_metric_events{metric_event_id, idempotency_key: $idempotency_key}",
            params,
            ScriptMutability::Immutable,
            "check cognitive metric idempotency key",
        )?;
        if let Some(row) = rows.rows.first() {
            return Err(CognitiveError::Metric(format!(
                "idempotency key `{}` already belongs to metric event `{}`",
                event.idempotency_key,
                str_col(row, 0)
            )));
        }
        Ok(())
    }

    fn append_ledger(&self, event: &CognitiveMetricEvent) -> Result<(), CognitiveError> {
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.ledger_path)?;
        writeln!(file, "{}", serde_json::to_string(event)?)?;
        Ok(())
    }
}

fn row_to_event(row: &[DataValue]) -> Result<CognitiveMetricEvent, CognitiveError> {
    let event_kind = str_col(row, 7);
    Ok(CognitiveMetricEvent {
        metric_event_id: str_col(row, 0),
        idempotency_key: str_col(row, 1),
        // Column 2 is the stored fingerprint; it is derived from the other
        // columns and is recomputed rather than round-tripped.
        metric_name: str_col(row, 3),
        metric_definition_version: int_col(row, 4),
        evaluation_dataset_version: str_col(row, 5),
        evaluation_window_id: str_col(row, 6),
        event_kind: event_kind.parse().map_err(|()| {
            CognitiveError::Metric(format!("unknown metric event kind `{event_kind}`"))
        })?,
        session_id: str_col(row, 8),
        turn_number: int_col(row, 9).max(0) as u64,
        cohort: crate::metrics::window::MetricCohort::new(
            str_col(row, 10),
            str_col(row, 11),
            str_col(row, 12),
        ),
        label_source: str_col(row, 13),
        outcome_status: str_col(row, 14),
        value: opt_float_col(row, 15),
        numerator: opt_float_col(row, 16),
        denominator: opt_float_col(row, 17),
        identities: json_col(row, 18)?,
        evidence_refs: json_col(row, 19)?,
        created_at: time_col(row, 20)?,
    })
}

fn event_params(
    event: &CognitiveMetricEvent,
    fingerprint: &str,
) -> Result<BTreeMap<String, DataValue>, CognitiveError> {
    let mut params = BTreeMap::new();
    let mut text = |key: &str, value: &str| {
        params.insert(key.to_string(), DataValue::from(value));
    };
    text("metric_event_id", &event.metric_event_id);
    text("idempotency_key", &event.idempotency_key);
    text("fingerprint", fingerprint);
    text("metric_name", &event.metric_name);
    text(
        "evaluation_dataset_version",
        &event.evaluation_dataset_version,
    );
    text("evaluation_window_id", &event.evaluation_window_id);
    text("event_kind", event.event_kind.as_str());
    text("session_id", &event.session_id);
    text("task_class", &event.cohort.task_class);
    text("model_id", &event.cohort.model_id);
    text("policy_version", &event.cohort.policy_version);
    text("label_source", &event.label_source);
    text("outcome_status", &event.outcome_status);
    text(
        "identities_json",
        &serde_json::to_string(&event.identities)?,
    );
    text(
        "evidence_refs_json",
        &serde_json::to_string(&event.evidence_refs)?,
    );
    text("created_at", &event.created_at.to_rfc3339());

    params.insert(
        "metric_definition_version".into(),
        DataValue::from(event.metric_definition_version),
    );
    params.insert(
        "turn_number".into(),
        DataValue::from(event.turn_number as i64),
    );
    params.insert("value".into(), opt_float_value(event.value));
    params.insert("numerator".into(), opt_float_value(event.numerator));
    params.insert("denominator".into(), opt_float_value(event.denominator));
    Ok(params)
}
