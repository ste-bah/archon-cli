#[path = "plan_store_authority.rs"]
mod plan_store_authority;
#[path = "plan_store_evidence.rs"]
mod plan_store_evidence;
#[path = "plan_store_materialization.rs"]
mod plan_store_materialization;
#[path = "plan_store_tasks.rs"]
mod plan_store_tasks;
#[path = "plan_store_transition.rs"]
mod plan_store_transition;
#[path = "plan_store_writes.rs"]
mod plan_store_writes;

use std::collections::BTreeMap;
use std::path::PathBuf;

use cozo::{DataValue, DbInstance, NamedRows, ScriptMutability};

use plan_store_materialization::{ensure_approving_decision, validate_canonical_task_generation};
use plan_store_writes::PlanWrite;

use crate::plan_models::{PersistedPlanTask, PlanApprovalRecord, PlanDocument, PlanStepStatus};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum PlanStoreIdentity {
    Physical(PathBuf),
    InMemory(String),
}

pub use plan_store_authority::PlanApprovalAuthority;

/// Persistence layer for plans using CozoDB.
///
/// Plans are stored as JSON blobs in a `plans` relation, keyed by session_id + plan_id.
#[derive(Clone)]
pub struct PlanStore {
    db: DbInstance,
    identity: PlanStoreIdentity,
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

fn plan_store_identity(
    db: &DbInstance,
    guard: &archon_cozo::CozoGuardConfig,
) -> Result<PlanStoreIdentity, std::io::Error> {
    if let Some(path) = guard.database_path() {
        return Ok(PlanStoreIdentity::Physical(path));
    }

    let identity = archon_cozo::in_memory_database_identity(db).ok_or_else(|| {
        std::io::Error::other("plan store requires a recognized database identity")
    })?;
    Ok(PlanStoreIdentity::InMemory(identity))
}

impl PlanStore {
    /// Open a plan store backed by an existing DbInstance (shared with session store).
    pub fn new(db: &DbInstance) -> Result<Self, std::io::Error> {
        let guard = archon_cozo::guarded_config_for(db).unwrap_or_default();
        let identity = plan_store_identity(db, &guard)?;
        let store = Self {
            db: db.clone(),
            identity,
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
        )?;
        self.create_relation(
            ":create plan_approval_roots {
                authority_id: String
                => verifier: String
            }",
            "plan store schema: create plan approval root relation",
        )?;
        self.create_relation(
            ":create plan_approval_authorities {
                session_id: String
                => verifier: String
            }",
            "plan store schema: create plan approval authorities relation",
        )?;
        self.create_relation(
            ":create plan_materializations {
                session_id: String,
                plan_id: String
                =>
                generation: String
            }",
            "plan store schema: create plan materializations relation",
        )?;
        self.create_relation(
            ":create plan_tasks {
                session_id: String,
                task_id: String
                =>
                plan_id: String,
                plan_step: Int,
                task_json: String,
                updated_at: String
            }",
            "plan store schema: create plan tasks relation",
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

    #[cfg(test)]
    pub(crate) fn materialization_claim_exists_for_test(
        &self,
        session_id: &str,
        plan_id: &str,
    ) -> Result<bool, std::io::Error> {
        let mut params = BTreeMap::new();
        params.insert("sid".into(), DataValue::from(session_id));
        params.insert("pid".into(), DataValue::from(plan_id));
        self.db
            .run_script(
                "?[plan_id] := *plan_materializations{session_id, plan_id}, session_id = $sid, plan_id = $pid",
                params,
                ScriptMutability::Immutable,
            )
            .map(|rows| !rows.rows.is_empty())
            .map_err(db_err)
    }

    pub fn is_same_store(&self, other: &Self) -> bool {
        self.identity == other.identity
    }

    /// Save a plan document.
    ///
    /// Materialized plans are immutable except through the task-status
    /// transactions, which update their mirrored plan steps atomically.
    pub fn save_plan(&self, session_id: &str, plan: &PlanDocument) -> Result<(), std::io::Error> {
        self.save_unmaterialized_plan(session_id, plan)
    }

    /// Save terminal plan, approval, and tasks atomically for an unclaimed plan.
    /// A claimed generation is immutable outside task-status transactions.
    pub fn save_terminal_plan_with_approval_and_tasks(
        &self,
        authority: &PlanApprovalAuthority,
        session_id: &str,
        plan: &PlanDocument,
        record: &PlanApprovalRecord,
        tasks: &[PersistedPlanTask],
    ) -> Result<(), std::io::Error> {
        validate_canonical_task_generation(plan, tasks)?;
        if record.session_id != session_id
            || record.plan_id != plan.id
            || plan.approval.as_ref() != Some(&record.approval)
        {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "plan approval record does not match terminal plan",
            ));
        }
        ensure_approving_decision(&record.approval)?;
        let transaction = self.db.multi_transaction(true);
        let result = (|| -> Result<(), std::io::Error> {
            self.require_authority_in(&transaction, authority, session_id)?;
            self.ensure_plan_unclaimed(&transaction, session_id, &plan.id)?;
            PlanWrite::materialization_claim(session_id, plan)?.run(&transaction)?;
            PlanWrite::plan(session_id, plan)?.run(&transaction)?;
            PlanWrite::approval(record)?.run(&transaction)?;
            tasks
                .iter()
                .try_for_each(|task| PlanWrite::task(session_id, task)?.run(&transaction))
        })();
        self.finish_transaction(transaction, result)
    }

    /// Persist a terminal plan and approval only while the plan is unclaimed.
    /// Claimed plans must use the task-bearing atomic approval path.
    pub fn save_terminal_plan_with_approval(
        &self,
        authority: &PlanApprovalAuthority,
        session_id: &str,
        plan: &PlanDocument,
        record: &PlanApprovalRecord,
    ) -> Result<(), std::io::Error> {
        if record.session_id != session_id
            || record.plan_id != plan.id
            || plan.approval.as_ref() != Some(&record.approval)
        {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "plan approval record does not match terminal plan",
            ));
        }

        let transaction = self.db.multi_transaction(true);
        let result = (|| -> Result<(), std::io::Error> {
            self.require_authority_in(&transaction, authority, session_id)?;
            self.ensure_plan_unclaimed(&transaction, session_id, &plan.id)?;
            PlanWrite::plan(session_id, plan)?.run(&transaction)?;
            PlanWrite::approval(record)?.run(&transaction)
        })();
        self.finish_transaction(transaction, result)
    }

    #[cfg(any(test, feature = "test-support"))]
    #[doc(hidden)]
    pub fn record_approval_event_for_test(
        &self,
        record: &PlanApprovalRecord,
    ) -> Result<(), std::io::Error> {
        let transaction = self.db.multi_transaction(true);
        let result = (|| -> Result<(), std::io::Error> {
            self.ensure_plan_unclaimed(&transaction, &record.session_id, &record.plan_id)?;
            PlanWrite::approval(record)?.run(&transaction)
        })();
        self.finish_transaction(transaction, result)
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

    /// Load every plan for a session.
    pub fn load_plans(&self, session_id: &str) -> Result<Vec<PlanDocument>, std::io::Error> {
        let mut params = BTreeMap::new();
        params.insert("sid".to_string(), DataValue::from(session_id));
        let result = self
            .db
            .run_script(
                "?[plan_json] := *plans{session_id, plan_json}, session_id = $sid",
                params,
                ScriptMutability::Immutable,
            )
            .map_err(db_err)?;
        result
            .rows
            .iter()
            .map(|row| {
                PlanDocument::from_json(row[0].get_str().unwrap_or("")).map_err(|error| {
                    std::io::Error::new(std::io::ErrorKind::InvalidData, error.to_string())
                })
            })
            .collect()
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

    /// Update a specific step's status without opening an unclaimed plan rewrite path.
    pub fn update_step_status(
        &self,
        session_id: &str,
        plan_id: &str,
        step_number: u32,
        status: PlanStepStatus,
    ) -> Result<(), std::io::Error> {
        let transaction = self.db.multi_transaction(true);
        let result = (|| -> Result<(), std::io::Error> {
            self.ensure_plan_unclaimed(&transaction, session_id, plan_id)?;
            let mut lookup = BTreeMap::new();
            lookup.insert("sid".to_string(), DataValue::from(session_id));
            lookup.insert("pid".to_string(), DataValue::from(plan_id));
            let rows = transaction
                .run_script(
                    "?[plan_json] := *plans{session_id, plan_id, plan_json}, session_id = $sid, plan_id = $pid",
                    lookup,
                )
                .map_err(db_err)?;
            let plan_json = rows
                .rows
                .first()
                .and_then(|row| row.first())
                .and_then(DataValue::get_str)
                .ok_or_else(|| {
                    std::io::Error::new(std::io::ErrorKind::NotFound, "plan not found")
                })?;
            let mut plan = PlanDocument::from_json(plan_json)
                .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
            let step = plan
                .steps
                .iter_mut()
                .find(|step| step.number == step_number)
                .ok_or_else(|| {
                    std::io::Error::new(std::io::ErrorKind::NotFound, "plan step not found")
                })?;
            step.status = status;

            let plan_json = plan.to_json();
            let updated_at = chrono::Utc::now().to_rfc3339();
            let mut params = BTreeMap::new();
            params.insert("session_id".to_string(), DataValue::from(session_id));
            params.insert("plan_id".to_string(), DataValue::from(plan_id));
            params.insert("plan_json".to_string(), DataValue::from(plan_json.as_str()));
            params.insert(
                "updated_at".to_string(),
                DataValue::from(updated_at.as_str()),
            );
            transaction
                .run_script(
                    "?[session_id, plan_id, plan_json, updated_at] <- [[$session_id, $plan_id, $plan_json, $updated_at]]
                     :put plans {session_id, plan_id => plan_json, updated_at}",
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
}
