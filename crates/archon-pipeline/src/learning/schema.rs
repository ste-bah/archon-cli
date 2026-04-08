//! CozoDB schema definitions for all learning subsystems.
//!
//! Defines 12 stored relations covering trajectories, patterns, causal graphs,
//! provenance tracking, episodic memory, DESC episode metadata, GNN weights,
//! training history, and shadow documents.

use anyhow::Result;
use cozo::ScriptMutability;

// ---------------------------------------------------------------------------
// CozoScript schema constants
// ---------------------------------------------------------------------------

pub const TRAJECTORIES_SCHEMA: &str = "
:create trajectories {
    trajectory_id: String
    =>
    route: String,
    agent_key: String,
    session_id: String,
    patterns: [String],
    context: [String],
    quality: Float,
    reward: Float,
    feedback_score: Float,
    weights_path: String,
    created_at: Int,
    updated_at: Int
}
";

pub const TRAJECTORY_STEPS_SCHEMA: &str = "
:create trajectory_steps {
    step_id: String
    =>
    trajectory_id: String,
    step_index: Int,
    action: String,
    observation: String,
    reward: Float,
    timestamp: Int
}
";

pub const PATTERNS_SCHEMA: &str = "
:create patterns {
    pattern_id: String
    =>
    pattern_type: String,
    description: String,
    embedding: [Float],
    frequency: Int,
    confidence: Float,
    created_at: Int,
    updated_at: Int
}
";

pub const CAUSAL_NODES_SCHEMA: &str = "
:create causal_nodes {
    node_id: String
    =>
    label: String,
    node_type: String,
    probability: Float,
    evidence_count: Int,
    created_at: Int
}
";

pub const CAUSAL_LINKS_SCHEMA: &str = "
:create causal_links {
    link_id: String
    =>
    source_ids: [String],
    target_id: String,
    strength: Float,
    link_type: String,
    created_at: Int
}
";

pub const PROVENANCE_SOURCES_SCHEMA: &str = "
:create provenance_sources {
    source_id: String
    =>
    source_type: String,
    uri: String,
    trust_score: Float,
    last_verified: Int,
    created_at: Int
}
";

pub const PROVENANCE_RECORDS_SCHEMA: &str = "
:create provenance_records {
    record_id: String
    =>
    source_id: String,
    entity_id: String,
    entity_type: String,
    derivation_chain: [String],
    confidence: Float,
    created_at: Int
}
";

pub const DESC_EPISODES_SCHEMA: &str = "
:create desc_episodes {
    episode_id: String
    =>
    session_id: String,
    description: String,
    outcome: String,
    reward: Float,
    tags: [String],
    created_at: Int
}
";

pub const DESC_EPISODE_METADATA_SCHEMA: &str = "
:create desc_episode_metadata {
    episode_id: String
    =>
    task_type: String,
    solution: String,
    quality_score: Float,
    trajectory_id: String,
    updated_at: Int
}
";

pub const GNN_WEIGHTS_SCHEMA: &str = "
:create gnn_weights {
    weight_id: String
    =>
    model_name: String,
    layer_index: Int,
    weights_blob: [Int],
    shape: [Int],
    created_at: Int
}
";

pub const TRAINING_HISTORY_SCHEMA: &str = "
:create training_history {
    run_id: String
    =>
    model_name: String,
    epochs: Int,
    epoch_losses: [Float],
    final_loss: Float,
    early_stopped: Bool,
    started_at: Int,
    finished_at: Int
}
";

pub const SHADOW_DOCUMENTS_SCHEMA: &str = "
:create shadow_documents {
    doc_id: String
    =>
    original_id: String,
    shadow_type: String,
    content: String,
    metadata: String,
    created_at: Int,
    updated_at: Int
}
";

// ---------------------------------------------------------------------------
// All relation names (used by both initialize and verify)
// ---------------------------------------------------------------------------

const ALL_LEARNING_RELATIONS: [&str; 12] = [
    "trajectories",
    "trajectory_steps",
    "patterns",
    "causal_nodes",
    "causal_links",
    "provenance_sources",
    "provenance_records",
    "desc_episodes",
    "desc_episode_metadata",
    "gnn_weights",
    "training_history",
    "shadow_documents",
];

// ---------------------------------------------------------------------------
// Schema verification report
// ---------------------------------------------------------------------------

/// Result of checking which learning relations exist in the database.
pub struct SchemaVerificationReport {
    /// Relations that were found in the database.
    pub present: Vec<String>,
    /// Relations that are expected but not found.
    pub missing: Vec<String>,
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Create all 12 learning relations in the database.
///
/// This function is idempotent: calling it when relations already exist will
/// silently succeed without error.
pub fn initialize_learning_schemas(db: &cozo::DbInstance) -> Result<()> {
    let schemas = [
        TRAJECTORIES_SCHEMA,
        TRAJECTORY_STEPS_SCHEMA,
        PATTERNS_SCHEMA,
        CAUSAL_NODES_SCHEMA,
        CAUSAL_LINKS_SCHEMA,
        PROVENANCE_SOURCES_SCHEMA,
        PROVENANCE_RECORDS_SCHEMA,
        DESC_EPISODES_SCHEMA,
        DESC_EPISODE_METADATA_SCHEMA,
        GNN_WEIGHTS_SCHEMA,
        TRAINING_HISTORY_SCHEMA,
        SHADOW_DOCUMENTS_SCHEMA,
    ];

    for script in schemas {
        match db.run_script(script, Default::default(), ScriptMutability::Mutable) {
            Ok(_) => {}
            Err(e) => {
                let msg = e.to_string();
                if msg.contains("already exists") || msg.contains("conflicts") {
                    // Relation already present — idempotent, skip.
                } else {
                    return Err(anyhow::anyhow!("Learning schema creation failed: {}", msg));
                }
            }
        }
    }
    Ok(())
}

/// Check which of the 12 learning relations exist in the database.
pub fn verify_learning_schemas(db: &cozo::DbInstance) -> Result<SchemaVerificationReport> {
    let result = db
        .run_script(
            "::relations",
            Default::default(),
            ScriptMutability::Immutable,
        )
        .map_err(|e| anyhow::anyhow!("Failed to list relations: {:?}", e))?;

    let name_col = result.headers.iter().position(|h| h == "name").unwrap_or(0);

    let existing: Vec<String> = result
        .rows
        .iter()
        .map(|row| row[name_col].get_str().unwrap_or_default().to_string())
        .collect();

    let mut present = Vec::new();
    let mut missing = Vec::new();

    for rel in &ALL_LEARNING_RELATIONS {
        if existing.contains(&rel.to_string()) {
            present.push(rel.to_string());
        } else {
            missing.push(rel.to_string());
        }
    }

    Ok(SchemaVerificationReport { present, missing })
}
