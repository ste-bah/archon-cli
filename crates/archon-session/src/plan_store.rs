use std::collections::BTreeMap;

use cozo::{DataValue, DbInstance, NamedRows, ScriptMutability};

use crate::plan_models::{PlanApprovalRecord, PlanDocument, PlanStepStatus};

/// Persistence layer for plans using CozoDB.
///
/// Plans are stored as JSON blobs in a `plans` relation, keyed by session_id + plan_id.
pub struct PlanStore {
    db: DbInstance,
    /// Write-guard config resolved from the *caller's* handle before it was
    /// cloned. `DbInstance::clone` produces a new value at a new address, and
    /// the guard registry keys on pointer identity, so the config has to be
    /// captured up front and carried explicitly.
    guard: archon_cozo::CozoGuardConfig,
}

fn db_err(e: impl std::fmt::Display) -> std::io::Error {
    std::io::Error::other(e.to_string())
}

fn empty_rows() -> NamedRows {
    NamedRows::new(vec![], vec![])
}

impl PlanStore {
    /// Open a plan store backed by an existing DbInstance (shared with session store).
    pub fn new(db: &DbInstance) -> Result<Self, std::io::Error> {
        let guard = archon_cozo::guarded_config_for(db).unwrap_or_default();
        let store = Self {
            db: db.clone(),
            guard,
        };
        store.init_schema()?;
        Ok(store)
    }

    fn run_mutable(
        &self,
        script: &str,
        params: BTreeMap<String, DataValue>,
        context: &str,
    ) -> anyhow::Result<NamedRows> {
        archon_cozo::run_script_guarded(
            &self.db,
            script,
            params,
            ScriptMutability::Mutable,
            context,
            &self.guard,
        )
    }

    fn init_schema(&self) -> Result<(), std::io::Error> {
        self.create_relation(
            ":create plans {
                session_id: String,
                plan_id: String
                =>
                plan_json: String,
                updated_at: String
            }",
            "plan store schema: create plans relation",
        )?;
        self.create_relation(
            ":create plan_approval_events {
                session_id: String,
                plan_id: String,
                decided_at: String
                =>
                approval_json: String
            }",
            "plan store schema: create plan approval events relation",
        )
    }

    fn create_relation(&self, schema: &str, context: &str) -> Result<(), std::io::Error> {
        self.run_mutable(schema, Default::default(), context)
            .or_else(|e| {
                let msg = e.to_string();
                if msg.contains("already exists") || msg.contains("conflicts") {
                    Ok(empty_rows())
                } else {
                    Err(db_err(e))
                }
            })?;
        Ok(())
    }

    /// Save a plan document.
    pub fn save_plan(&self, session_id: &str, plan: &PlanDocument) -> Result<(), std::io::Error> {
        let now = chrono::Utc::now().to_rfc3339();
        let json = plan.to_json();
        let mut params = BTreeMap::new();
        params.insert("session_id".to_string(), DataValue::from(session_id));
        params.insert("plan_id".to_string(), DataValue::from(plan.id.as_str()));
        params.insert("plan_json".to_string(), DataValue::from(json.as_str()));
        params.insert("updated_at".to_string(), DataValue::from(now.as_str()));
        self.run_mutable(
            "?[session_id, plan_id, plan_json, updated_at] <- [[$session_id, $plan_id, $plan_json, $updated_at]]
             :put plans {session_id, plan_id => plan_json, updated_at}",
            params,
            "plan store: save plan document",
        )
        .map_err(db_err)?;
        Ok(())
    }

    /// Append an immutable approval decision and save the terminal plan in one
    /// Cozo transaction, so no reader can observe one without the other.
    pub fn save_terminal_plan_with_approval(
        &self,
        session_id: &str,
        plan: &PlanDocument,
        record: &PlanApprovalRecord,
    ) -> Result<(), std::io::Error> {
        if record.session_id != session_id || record.plan_id != plan.id {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "plan approval record does not match terminal plan",
            ));
        }

        let plan_json = plan.to_json();
        let approval_json = serde_json::to_string(&record.approval).map_err(db_err)?;
        let updated_at = chrono::Utc::now().to_rfc3339();
        let mut params = BTreeMap::new();
        params.insert("session_id".to_string(), DataValue::from(session_id));
        params.insert("plan_id".to_string(), DataValue::from(plan.id.as_str()));
        params.insert("plan_json".to_string(), DataValue::from(plan_json.as_str()));
        params.insert(
            "updated_at".to_string(),
            DataValue::from(updated_at.as_str()),
        );
        params.insert(
            "decided_at".to_string(),
            DataValue::from(record.approval.decided_at.as_str()),
        );
        params.insert(
            "approval_json".to_string(),
            DataValue::from(approval_json.as_str()),
        );

        let transaction = self.db.multi_transaction(true);
        let result = (|| -> Result<(), std::io::Error> {
            transaction
                .run_script(
                    "?[session_id, plan_id, plan_json, updated_at] <- [[$session_id, $plan_id, $plan_json, $updated_at]]
                     :put plans {session_id, plan_id => plan_json, updated_at}",
                    params.clone(),
                )
                .map_err(db_err)?;
            transaction
                .run_script(
                    "?[session_id, plan_id, decided_at, approval_json] <- [[$session_id, $plan_id, $decided_at, $approval_json]]
                     :insert plan_approval_events {session_id, plan_id, decided_at => approval_json}",
                    params,
                )
                .map_err(db_err)?;
            Ok(())
        })();
        if result.is_ok() {
            transaction.commit().map_err(db_err)?;
        } else {
            let _ = transaction.abort();
        }
        result
    }

    /// Append an immutable approval decision to the plan's durable audit ledger.
    pub fn record_approval_event(&self, record: &PlanApprovalRecord) -> Result<(), std::io::Error> {
        let approval_json = serde_json::to_string(&record.approval).map_err(db_err)?;
        let mut params = BTreeMap::new();
        params.insert(
            "session_id".to_string(),
            DataValue::from(record.session_id.as_str()),
        );
        params.insert(
            "plan_id".to_string(),
            DataValue::from(record.plan_id.as_str()),
        );
        params.insert(
            "decided_at".to_string(),
            DataValue::from(record.approval.decided_at.as_str()),
        );
        params.insert(
            "approval_json".to_string(),
            DataValue::from(approval_json.as_str()),
        );
        self.run_mutable(
            "?[session_id, plan_id, decided_at, approval_json] <- [[$session_id, $plan_id, $decided_at, $approval_json]]
             :insert plan_approval_events {session_id, plan_id, decided_at => approval_json}",
            params,
            "plan store: record approval event",
        )
        .map_err(db_err)?;
        Ok(())
    }

    /// Load every recorded approval decision for a plan in chronological order.
    pub fn load_approval_events(
        &self,
        session_id: &str,
        plan_id: &str,
    ) -> Result<Vec<PlanApprovalRecord>, std::io::Error> {
        let mut params = BTreeMap::new();
        params.insert("sid".to_string(), DataValue::from(session_id));
        params.insert("pid".to_string(), DataValue::from(plan_id));
        let result = self
            .db
            .run_script(
                "?[decided_at, approval_json] := *plan_approval_events{session_id, plan_id, decided_at, approval_json}, session_id = $sid, plan_id = $pid :sort decided_at",
                params,
                ScriptMutability::Immutable,
            )
            .map_err(db_err)?;
        result
            .rows
            .iter()
            .map(|row| {
                let approval_json = row[1].get_str().unwrap_or("");
                let approval = serde_json::from_str(approval_json).map_err(db_err)?;
                Ok(PlanApprovalRecord {
                    plan_id: plan_id.to_string(),
                    session_id: session_id.to_string(),
                    approval,
                })
            })
            .collect()
    }

    /// Load a plan by session_id and plan_id.
    pub fn load_plan(
        &self,
        session_id: &str,
        plan_id: &str,
    ) -> Result<Option<PlanDocument>, std::io::Error> {
        let mut params = BTreeMap::new();
        params.insert("sid".to_string(), DataValue::from(session_id));
        params.insert("pid".to_string(), DataValue::from(plan_id));
        let result = self
            .db
            .run_script(
                "?[plan_json] := *plans{session_id, plan_id, plan_json}, session_id = $sid, plan_id = $pid",
                params,
                ScriptMutability::Immutable,
            )
            .map_err(db_err)?;
        self.decode_first_plan(result)
    }

    /// Load the most recent plan for a session.
    pub fn load_latest_plan(
        &self,
        session_id: &str,
    ) -> Result<Option<PlanDocument>, std::io::Error> {
        let mut params = BTreeMap::new();
        params.insert("sid".to_string(), DataValue::from(session_id));
        let result = self
            .db
            .run_script(
                "?[plan_json, updated_at] := *plans{session_id, plan_json, updated_at}, session_id = $sid :sort -updated_at :limit 1",
                params,
                ScriptMutability::Immutable,
            )
            .map_err(db_err)?;
        self.decode_first_plan(result)
    }

    fn decode_first_plan(&self, result: NamedRows) -> Result<Option<PlanDocument>, std::io::Error> {
        let Some(row) = result.rows.first() else {
            return Ok(None);
        };
        PlanDocument::from_json(row[0].get_str().unwrap_or(""))
            .map(Some)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))
    }

    /// Update a specific step's status.
    pub fn update_step_status(
        &self,
        session_id: &str,
        plan_id: &str,
        step_number: u32,
        status: PlanStepStatus,
    ) -> Result<(), std::io::Error> {
        let mut plan = self
            .load_plan(session_id, plan_id)?
            .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "plan not found"))?;
        if let Some(step) = plan.steps.iter_mut().find(|s| s.number == step_number) {
            step.status = status;
        }
        self.save_plan(session_id, &plan)
    }
}
