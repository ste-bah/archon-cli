//! Agent-callable governed-learning tools required by the Evidence Engine.

use async_trait::async_trait;
use serde_json::{Value, json};

use crate::evidence_cli;
use crate::tool::{PermissionLevel, Tool, ToolContext, ToolResult};

pub const LEARNING_TOOL_NAMES: &[&str] = &[
    "LearningStatus",
    "LearningInspect",
    "BehaviourProposals",
    "BehaviourApprove",
    "BehaviourRollback",
];

macro_rules! learning_tool {
    ($struct_name:ident, $name:literal, $desc:literal, $schema:ident, $args:ident, $perm:expr) => {
        pub struct $struct_name;

        #[async_trait]
        impl Tool for $struct_name {
            fn name(&self) -> &str {
                $name
            }

            fn description(&self) -> &str {
                $desc
            }

            fn input_schema(&self) -> Value {
                $schema()
            }

            async fn execute(&self, input: Value, ctx: &ToolContext) -> ToolResult {
                match $args(&input) {
                    Ok(args) => evidence_cli::run_archon(args, ctx).await,
                    Err(e) => ToolResult::error(e),
                }
            }

            fn permission_level(&self, _input: &Value) -> PermissionLevel {
                $perm
            }
        }
    };
}

learning_tool!(
    LearningStatus,
    "LearningStatus",
    "Show governed-learning status and behaviour proposal counts.",
    empty_schema,
    status_args,
    PermissionLevel::Safe
);
learning_tool!(
    LearningInspect,
    "LearningInspect",
    "Inspect a learning event, behaviour proposal, or manifest version by id.",
    id_schema,
    inspect_args,
    PermissionLevel::Safe
);
learning_tool!(
    BehaviourProposals,
    "BehaviourProposals",
    "List pending governed-learning behaviour proposals.",
    empty_schema,
    proposals_args,
    PermissionLevel::Safe
);
learning_tool!(
    BehaviourApprove,
    "BehaviourApprove",
    "Approve and apply a pending governed-learning behaviour proposal.",
    proposal_schema,
    approve_args,
    PermissionLevel::Risky
);
learning_tool!(
    BehaviourRollback,
    "BehaviourRollback",
    "Rollback a behaviour manifest to a previous version.",
    rollback_schema,
    rollback_args,
    PermissionLevel::Risky
);

fn status_args(_input: &Value) -> Result<Vec<String>, String> {
    Ok(vec!["behaviour".into(), "status".into()])
}

fn inspect_args(input: &Value) -> Result<Vec<String>, String> {
    Ok(vec![
        "behaviour".into(),
        "show".into(),
        evidence_cli::required_string(input, "id")?,
    ])
}

fn proposals_args(_input: &Value) -> Result<Vec<String>, String> {
    Ok(vec!["behaviour".into(), "proposals".into()])
}

fn approve_args(input: &Value) -> Result<Vec<String>, String> {
    Ok(vec![
        "behaviour".into(),
        "approve".into(),
        evidence_cli::required_string(input, "proposal_id")?,
    ])
}

fn rollback_args(input: &Value) -> Result<Vec<String>, String> {
    let mut args = vec![
        "behaviour".into(),
        "rollback".into(),
        evidence_cli::required_string(input, "version_id")?,
    ];
    if let Some(reason) = evidence_cli::opt_string(input, "reason") {
        args.push("--reason".into());
        args.push(reason);
    }
    Ok(args)
}

fn empty_schema() -> Value {
    evidence_cli::object_schema(json!({}), &[])
}

fn id_schema() -> Value {
    evidence_cli::object_schema(json!({ "id": { "type": "string" } }), &["id"])
}

fn proposal_schema() -> Value {
    evidence_cli::object_schema(
        json!({ "proposal_id": { "type": "string" } }),
        &["proposal_id"],
    )
}

fn rollback_schema() -> Value {
    evidence_cli::object_schema(
        json!({
            "version_id": { "type": "string" },
            "reason": { "type": "string" }
        }),
        &["version_id"],
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_learning_tool_names_match_tspec_surface() {
        assert_eq!(LEARNING_TOOL_NAMES.len(), 5);
        assert!(LEARNING_TOOL_NAMES.contains(&"LearningStatus"));
        assert!(LEARNING_TOOL_NAMES.contains(&"BehaviourRollback"));
    }

    #[test]
    fn test_rollback_preserves_reason_argument() {
        let args = rollback_args(&json!({
            "version_id": "manifest-v1",
            "reason": "bad retrieval profile"
        }))
        .unwrap();
        assert_eq!(
            args,
            vec![
                "behaviour",
                "rollback",
                "manifest-v1",
                "--reason",
                "bad retrieval profile"
            ]
        );
    }

    #[test]
    fn test_approve_rejects_empty_proposal_id() {
        let err = approve_args(&json!({ "proposal_id": "" })).unwrap_err();
        assert!(err.contains("proposal_id is required"));
    }
}
