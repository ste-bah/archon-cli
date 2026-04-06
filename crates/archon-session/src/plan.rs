use std::collections::BTreeMap;

use cozo::{DataValue, DbInstance, NamedRows, ScriptMutability};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PlanStepStatus {
    Pending,
    InProgress,
    Complete,
    Skipped,
}

impl std::fmt::Display for PlanStepStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Pending => write!(f, "pending"),
            Self::InProgress => write!(f, "in_progress"),
            Self::Complete => write!(f, "complete"),
            Self::Skipped => write!(f, "skipped"),
        }
    }
}

impl PlanStepStatus {
    pub fn from_str(s: &str) -> Self {
        match s {
            "in_progress" => Self::InProgress,
            "complete" => Self::Complete,
            "skipped" => Self::Skipped,
            _ => Self::Pending,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanStep {
    pub number: u32,
    pub description: String,
    pub affected_files: Vec<String>,
    pub status: PlanStepStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanDocument {
    pub id: String,
    pub title: String,
    pub steps: Vec<PlanStep>,
    pub risks: Vec<String>,
    pub questions: Vec<String>,
    pub status: String, // "draft", "active", "complete", "abandoned"
}

impl PlanDocument {
    pub fn new(id: &str, title: &str) -> Self {
        Self {
            id: id.to_string(),
            title: title.to_string(),
            steps: Vec::new(),
            risks: Vec::new(),
            questions: Vec::new(),
            status: "draft".to_string(),
        }
    }

    /// Serialize the plan to JSON for storage.
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }

    /// Deserialize from JSON.
    pub fn from_json(json: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(json)
    }

    /// Get completion percentage.
    pub fn completion_pct(&self) -> f32 {
        if self.steps.is_empty() {
            return 0.0;
        }
        let done = self
            .steps
            .iter()
            .filter(|s| matches!(s.status, PlanStepStatus::Complete | PlanStepStatus::Skipped))
            .count();
        (done as f32 / self.steps.len() as f32) * 100.0
    }

    /// Format as human-readable text for injection into context.
    pub fn to_context_string(&self) -> String {
        let mut out = format!(
            "## Plan: {}\nStatus: {} ({:.0}% complete)\n\n",
            self.title,
            self.status,
            self.completion_pct()
        );
        for step in &self.steps {
            let marker = match step.status {
                PlanStepStatus::Complete => "[x]",
                PlanStepStatus::InProgress => "[>]",
                PlanStepStatus::Skipped => "[-]",
                PlanStepStatus::Pending => "[ ]",
            };
            out.push_str(&format!("{} {}. {}\n", marker, step.number, step.description));
            if !step.affected_files.is_empty() {
                out.push_str(&format!("    Files: {}\n", step.affected_files.join(", ")));
            }
        }
        if !self.risks.is_empty() {
            out.push_str("\nRisks:\n");
            for r in &self.risks {
                out.push_str(&format!("  - {r}\n"));
            }
        }
        if !self.questions.is_empty() {
            out.push_str("\nOpen questions:\n");
            for q in &self.questions {
                out.push_str(&format!("  - {q}\n"));
            }
        }
        out
    }
}

/// Persistence layer for plans using CozoDB.
///
/// Plans are stored as JSON blobs in a `plans` relation, keyed by session_id + plan_id.
pub struct PlanStore {
    db: DbInstance,
}

fn db_err(e: impl std::fmt::Display) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::Other, e.to_string())
}

fn empty_rows() -> NamedRows {
    NamedRows::new(vec![], vec![])
}

impl PlanStore {
    /// Open a plan store backed by an existing DbInstance (shared with session store).
    pub fn new(db: &DbInstance) -> Result<Self, std::io::Error> {
        let store = Self { db: db.clone() };
        store.init_schema()?;
        Ok(store)
    }

    fn init_schema(&self) -> Result<(), std::io::Error> {
        self.db
            .run_script(
                ":create plans {
                session_id: String,
                plan_id: String
                =>
                plan_json: String,
                updated_at: String
            }",
                Default::default(),
                ScriptMutability::Mutable,
            )
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

        self.db
            .run_script(
                "?[session_id, plan_id, plan_json, updated_at] <- [[$session_id, $plan_id, $plan_json, $updated_at]]
             :put plans {session_id, plan_id => plan_json, updated_at}",
                params,
                ScriptMutability::Mutable,
            )
            .map_err(db_err)?;

        Ok(())
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

        if result.rows.is_empty() {
            return Ok(None);
        }

        let json_str = result.rows[0][0].get_str().unwrap_or("");
        match PlanDocument::from_json(json_str) {
            Ok(plan) => Ok(Some(plan)),
            Err(e) => Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                e.to_string(),
            )),
        }
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

        if result.rows.is_empty() {
            return Ok(None);
        }

        let json_str = result.rows[0][0].get_str().unwrap_or("");
        match PlanDocument::from_json(json_str) {
            Ok(plan) => Ok(Some(plan)),
            Err(e) => Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                e.to_string(),
            )),
        }
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

/// Build a plan context string suitable for injection into compaction summaries.
/// Returns None if no active plan exists.
pub fn plan_context_for_compaction(store: &PlanStore, session_id: &str) -> Option<String> {
    match store.load_latest_plan(session_id) {
        Ok(Some(plan)) if plan.status == "active" || plan.status == "draft" => Some(format!(
            "\n\n---\n[Active Plan]\n{}",
            plan.to_context_string()
        )),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_db() -> DbInstance {
        DbInstance::new("mem", "", "").expect("in-memory db")
    }

    #[test]
    fn plan_roundtrip() {
        let db = test_db();
        let store = PlanStore::new(&db).expect("init");

        let mut plan = PlanDocument::new("plan-1", "Test Plan");
        plan.steps.push(PlanStep {
            number: 1,
            description: "First step".to_string(),
            affected_files: vec!["src/main.rs".to_string()],
            status: PlanStepStatus::Pending,
        });
        plan.steps.push(PlanStep {
            number: 2,
            description: "Second step".to_string(),
            affected_files: vec![],
            status: PlanStepStatus::Pending,
        });
        plan.risks.push("Might break things".to_string());

        store.save_plan("sess1", &plan).expect("save");

        let loaded = store
            .load_plan("sess1", "plan-1")
            .expect("load")
            .expect("found");
        assert_eq!(loaded.title, "Test Plan");
        assert_eq!(loaded.steps.len(), 2);
        assert_eq!(loaded.steps[0].description, "First step");
        assert_eq!(loaded.risks.len(), 1);
    }

    #[test]
    fn update_step_status() {
        let db = test_db();
        let store = PlanStore::new(&db).expect("init");

        let mut plan = PlanDocument::new("plan-2", "Step Test");
        plan.steps.push(PlanStep {
            number: 1,
            description: "Do something".to_string(),
            affected_files: vec![],
            status: PlanStepStatus::Pending,
        });
        store.save_plan("sess1", &plan).expect("save");

        store
            .update_step_status("sess1", "plan-2", 1, PlanStepStatus::Complete)
            .expect("update");

        let loaded = store
            .load_plan("sess1", "plan-2")
            .expect("load")
            .expect("found");
        assert_eq!(loaded.steps[0].status, PlanStepStatus::Complete);
    }

    #[test]
    fn completion_percentage() {
        let mut plan = PlanDocument::new("p", "t");
        assert_eq!(plan.completion_pct(), 0.0);

        plan.steps.push(PlanStep {
            number: 1,
            description: "a".into(),
            affected_files: vec![],
            status: PlanStepStatus::Complete,
        });
        plan.steps.push(PlanStep {
            number: 2,
            description: "b".into(),
            affected_files: vec![],
            status: PlanStepStatus::Pending,
        });
        assert!((plan.completion_pct() - 50.0).abs() < 0.1);

        plan.steps[1].status = PlanStepStatus::Skipped;
        assert!((plan.completion_pct() - 100.0).abs() < 0.1);
    }

    #[test]
    fn to_context_string_format() {
        let mut plan = PlanDocument::new("p", "My Plan");
        plan.status = "active".to_string();
        plan.steps.push(PlanStep {
            number: 1,
            description: "Step one".into(),
            affected_files: vec!["a.rs".into()],
            status: PlanStepStatus::Complete,
        });
        plan.steps.push(PlanStep {
            number: 2,
            description: "Step two".into(),
            affected_files: vec![],
            status: PlanStepStatus::Pending,
        });

        let ctx = plan.to_context_string();
        assert!(ctx.contains("My Plan"));
        assert!(ctx.contains("[x] 1. Step one"));
        assert!(ctx.contains("[ ] 2. Step two"));
        assert!(ctx.contains("a.rs"));
    }

    #[test]
    fn load_nonexistent_returns_none() {
        let db = test_db();
        let store = PlanStore::new(&db).expect("init");
        let result = store.load_plan("sess1", "nope").expect("load");
        assert!(result.is_none());
    }
}
