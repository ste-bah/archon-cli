//! Reading spec shapes that older builds wrote.
//!
//! Owns the hand-written `Deserialize` implementations for [`StageKind`] and
//! [`WorkflowSpec`] — the two places where the wire format is deliberately
//! wider than the type. Both exist for the same reason: run state
//! (`state.json`) embeds a full spec and saved templates embed the spec YAML,
//! so anything an earlier build persisted must keep loading.
//!
//! Keeping them here rather than beside the types means every backward
//! compatibility concession is in one file, and the type definitions in
//! [`crate::spec`] read as the current shape rather than the historical one.

use std::collections::BTreeMap;

use serde::Deserialize;

use crate::spec::{StageKind, StageSpec, WorkflowSpec};

impl<'de> Deserialize<'de> for StageKind {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = String::deserialize(deserializer)?;
        match raw.as_str() {
            "agent" => Ok(StageKind::Agent),
            "fanout" => Ok(StageKind::Fanout),
            "reduce" => Ok(StageKind::Reduce),
            "tool" => Ok(StageKind::Tool),
            "checkpoint" => Ok(StageKind::Checkpoint),
            "quality_gate" => Ok(StageKind::QualityGate),
            "human_gate" => Ok(StageKind::HumanGate),
            "implementation" => Ok(StageKind::Implementation),
            // Back-compat: `condition` stages never branched. No evaluator was
            // ever wired up, so a condition stage always proceeded — exactly
            // what a checkpoint does. Persisted runs keep loading, and they
            // keep behaving the way they always did.
            "condition" => Ok(StageKind::Checkpoint),
            other => Err(serde::de::Error::unknown_variant(
                other,
                &[
                    "agent",
                    "fanout",
                    "reduce",
                    "tool",
                    "checkpoint",
                    "quality_gate",
                    "human_gate",
                    "implementation",
                ],
            )),
        }
    }
}

/// Deserialization shadow for [`WorkflowSpec`].
///
/// `WorkflowSpec` used to carry `provider_tiers`, `artifact_policy`, and
/// `quality_gates`. Nothing ever read them, so they were removed. They cannot
/// simply vanish from the wire format: run state (`state.json`) embeds a full
/// `WorkflowSpec` and saved templates embed the spec YAML, so every run and
/// template written by an earlier build still carries those three keys. This
/// shadow accepts and discards them, which keeps `deny_unknown_fields` doing
/// its real job — catching typos in hand-authored specs — while old runs keep
/// loading.
///
/// Keep the live fields here in sync with `WorkflowSpec` itself.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WorkflowSpecDe {
    schema: String,
    name: String,
    #[serde(default)]
    task: String,
    #[serde(default)]
    target_repository_root: Option<String>,
    #[serde(default = "crate::spec::default_max_parallelism")]
    max_parallelism: u32,
    #[serde(default = "crate::spec::default_max_agents")]
    max_agents: u32,
    stages: Vec<StageSpec>,
    #[serde(
        default,
        deserialize_with = "crate::spec_deser::deserialize_permissions"
    )]
    permissions: BTreeMap<String, serde_json::Value>,
    #[serde(
        default,
        deserialize_with = "crate::spec_deser::deserialize_learning_hooks"
    )]
    learning_hooks: Vec<String>,

    // Accepted and discarded. See the doc comment above. Never read by
    // construction — the point is to consume the key, not the value.
    #[serde(default)]
    #[allow(dead_code)]
    provider_tiers: serde::de::IgnoredAny,
    #[serde(default)]
    #[allow(dead_code)]
    artifact_policy: serde::de::IgnoredAny,
    #[serde(default)]
    #[allow(dead_code)]
    quality_gates: serde::de::IgnoredAny,
}

impl<'de> Deserialize<'de> for WorkflowSpec {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let WorkflowSpecDe {
            schema,
            name,
            task,
            target_repository_root,
            max_parallelism,
            max_agents,
            stages,
            permissions,
            learning_hooks,
            provider_tiers: _,
            artifact_policy: _,
            quality_gates: _,
        } = WorkflowSpecDe::deserialize(deserializer)?;
        Ok(Self {
            schema,
            name,
            task,
            target_repository_root,
            max_parallelism,
            max_agents,
            stages,
            permissions,
            learning_hooks,
        })
    }
}
