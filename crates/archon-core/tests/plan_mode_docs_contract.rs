//! Contract tests for the user-visible Plan Mode lifecycle reference.
//!
//! These tests read the shipped documentation and configuration template from
//! disk, so Plan Mode forms, artifacts, and policy defaults cannot silently
//! drift from their implemented contracts.

use archon_core::{agent::plan_approval::noninteractive_decision, config::ContextConfig};
use archon_session::plan::PlanApprovalDecision;
use archon_tools::plan_mode::PLAN_MODE_SAFE_TOOLS;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}

fn read(relative: &str) -> String {
    let path = repo_root().join(relative);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("read {}: {error}", path.display()))
}

fn documented_plan_safe_tools(tools: &str) -> BTreeSet<String> {
    let section = tools
        .split("## Plan Mode safe tools")
        .nth(1)
        .and_then(|rest| rest.split("## Planning & isolation").next())
        .expect("tools reference must contain the Plan Mode safe tools section");

    section
        .lines()
        .filter_map(|line| {
            let mut columns = line.split('|').map(str::trim);
            let _ = columns.next();
            let tool = columns.next()?;
            tool.strip_prefix('`')?.strip_suffix('`').map(str::to_owned)
        })
        .collect()
}

#[test]
fn plan_mode_references_describe_the_process_state_trust_boundary() {
    let references = [
        ("tools", read("docs/reference/tools.md")),
        ("permissions", read("docs/reference/permissions.md")),
        ("slash commands", read("docs/reference/slash-commands.md")),
        ("configuration", read("docs/reference/config.md")),
    ];
    let trust_boundary = "Plan Mode blocks working-tree mutations by default while retaining explicit process-state controls: `TaskCreate`, `TaskUpdate`, and `Agent`.";

    for (name, reference) in references {
        assert!(
            reference.contains(trust_boundary),
            "{name} reference must state the Plan Mode trust boundary"
        );
        assert!(
            reference.contains(
                "Agent model/tool actions remain subject to Plan Mode and preflight boundaries"
            ),
            "{name} reference must describe Agent's Plan Mode boundary"
        );
        assert_no_global_plan_mode_claims(name, &reference);
        assert!(
            !reference.contains("read-only plan") && !reference.contains("read-only Plan Mode"),
            "{name} reference must not describe Plan Mode as a read-only plan"
        );
    }
}

#[test]
fn plan_mode_docs_reject_global_read_only_and_no_mutation_claims() {
    for claim in [
        "Plan Mode is read-only.",
        "Plan Mode allows no mutations.",
        "Plan Mode does not permit mutations.",
        "`plan` (read-only)",
    ] {
        assert!(
            is_global_plan_mode_claim(claim),
            "regression guard must reject global Plan Mode claim: {claim}"
        );
    }
}

#[test]
fn production_plan_mode_wording_preserves_the_canonical_trust_boundary() {
    let sources = [
        (
            "Plan Mode tool",
            read("crates/archon-tools/src/plan_mode.rs"),
        ),
        ("agent mode", read("crates/archon-tools/src/tool.rs")),
        (
            "model reminder",
            read("crates/archon-core/src/agent/memory_integration.rs"),
        ),
        (
            "preflight rejection",
            read("crates/archon-core/src/agent/tool_preflight_gates.rs"),
        ),
        (
            "dispatch rejection",
            read("crates/archon-core/src/dispatch.rs"),
        ),
    ];

    for (name, source) in sources {
        assert!(
            source.contains("Plan Mode blocks working-tree mutations by default"),
            "{name} must describe the working-tree mutation restriction"
        );
        assert!(
            source.contains("canonical Plan-safe"),
            "{name} must identify the canonical Plan-safe allowlist"
        );
        for control in ["TaskCreate", "TaskUpdate", "Agent"] {
            assert!(
                source.contains(control),
                "{name} must identify retained Plan-safe control {control}"
            );
        }
        for misleading_claim in [
            "only read-only tools",
            "Plan Mode is read-only",
            "Plan Mode allows no mutations",
            "Plan Mode does not permit mutations",
        ] {
            assert!(
                !source
                    .to_ascii_lowercase()
                    .contains(&misleading_claim.to_ascii_lowercase()),
                "{name} must not make global Plan Mode claim: {misleading_claim}"
            );
        }
    }
}
#[test]
fn plan_mode_docs_allowlist_exactly_matches_production() {
    let documented = documented_plan_safe_tools(&read("docs/reference/tools.md"));
    let production = PLAN_MODE_SAFE_TOOLS
        .iter()
        .map(|tool| (*tool).to_owned())
        .collect::<BTreeSet<_>>();

    assert_eq!(documented, production);
}

fn assert_no_global_plan_mode_claims(name: &str, reference: &str) {
    for line in reference.lines() {
        assert!(
            !is_global_plan_mode_claim(line),
            "{name} reference must not make global Plan Mode read-only/no-mutation claim: {line}"
        );
    }
}

fn is_global_plan_mode_claim(text: &str) -> bool {
    let normalized = text
        .to_ascii_lowercase()
        .replace(['`', '*', '_'], "")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");

    [
        "plan mode is read-only",
        "plan mode is a read-only",
        "plan mode allows no mutations",
        "plan mode has no mutations",
        "plan mode does not permit mutations",
        "plan (read-only)",
        "plan is read-only",
    ]
    .iter()
    .any(|claim| normalized.contains(claim))
}

#[test]
fn plan_mode_reference_documents_supported_forms_and_lifecycle() {
    let slash = read("docs/reference/slash-commands.md");
    let permissions = read("docs/reference/permissions.md");
    let tools = read("docs/reference/tools.md");
    let implementation = read("src/command/plan.rs");

    for (documented, implemented) in [
        ("`/plan`", "\"\" | \"show\""),
        ("`/plan show`", "\"\" | \"show\""),
        ("`/plan open`", "\"open\""),
        ("`/plan off|exit|done`", "\"off\" | \"exit\" | \"done\""),
    ] {
        assert!(
            implementation.contains(implemented),
            "PlanHandler no longer implements documented form {documented}"
        );
        assert!(
            slash.contains(documented),
            "slash command reference is missing {documented}"
        );
    }

    let artifact_root = tempfile::tempdir().unwrap();
    let plan_path =
        archon_core::plan_file::plan_document_path(artifact_root.path(), "plan-id").unwrap();
    let audit_path =
        archon_core::plan_file::plan_audit_path(artifact_root.path(), "session-id").unwrap();
    let plan_extension = plan_path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap();
    let audit_extension = audit_path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap();
    let plan_artifact = format!(".archon/plans/<plan-id>.{plan_extension}");
    let audit_artifact = format!(".archon/plan-audit/<session-id>.{audit_extension}");

    for claim in [
        "three outcomes: approve, reject, or revise",
        "rejects exit and keeps mutating tools blocked",
        "safe restoration",
        "plan-linked task rows persist and rehydrate",
        "unrelated manual tasks remain process-scoped",
        "completed, omitted, deviated, and unplanned-extra",
        &plan_artifact,
        &audit_artifact,
    ] {
        assert!(
            slash.contains(claim),
            "slash command reference is missing lifecycle claim: {claim}"
        );
    }
    assert!(
        permissions.contains("noninteractive Plan Mode approval defaults to approve"),
        "permissions reference must state the noninteractive approval default"
    );
    for claim in [
        "`EnterPlanMode`",
        "`ExitPlanMode`",
        "Plan-safe allowlist",
        "`TaskCreate`",
        "`TaskUpdate`",
        "`TaskGet`",
        "`TaskList`",
        "`Agent`",
        "Other task and agent controls remain denied unless explicitly allowlisted",
        "mutating tools remain denied",
        "evidence requirements block completion",
    ] {
        assert!(
            tools.contains(claim),
            "tools reference is missing Plan Mode claim: {claim}"
        );
    }
}

#[test]
fn plan_mode_reference_distinguishes_structured_and_user_slash_exit_paths() {
    let permissions = read("docs/reference/permissions.md");
    let slash = read("docs/reference/slash-commands.md");
    let implementation = read("src/command/plan.rs");
    let structured_exit =
        "A structured `ExitPlanMode` submission requires approval before Plan Mode is exited.";
    let slash_exit = "`/plan off`, `/plan exit`, and `/plan done` are explicit user commands that exit Plan Mode directly and restore `default` without structured plan approval.";

    for (name, reference) in [("permissions", permissions), ("slash commands", slash)] {
        assert!(
            reference.contains(structured_exit),
            "{name} reference must distinguish the approved structured exit path"
        );
        assert!(
            reference.contains(slash_exit),
            "{name} reference must distinguish the direct user slash exit path"
        );
        assert!(
            !reference.contains(
                "Its exact allowlist remains in effect until a structured plan is submitted through `ExitPlanMode`"
            ),
            "{name} reference must not imply every Plan Mode exit requires structured approval"
        );
    }

    assert!(implementation.contains("\"off\" | \"exit\" | \"done\" => Self::exit_plan(ctx)"));
    assert!(implementation.contains("CommandEffect::SetPermissionMode(\"default\".to_string())"));
}

#[test]
fn plan_mode_template_has_exact_structured_policy_values() {
    let template = read("config.toml");
    let config = read("docs/reference/config.md");
    let parsed = template.parse::<toml::Value>().unwrap();
    let context = parsed
        .get("context")
        .and_then(toml::Value::as_table)
        .expect("config.toml must have a [context] table");

    assert!(
        !context.contains_key("plan_model"),
        "plan_model must remain an optional commented template override"
    );
    assert_eq!(
        context
            .get("noninteractive_plan_approval")
            .and_then(toml::Value::as_str),
        Some("approve"),
        "template must set the exact default approval policy"
    );
    assert!(ContextConfig::default().plan_model.is_none());
    assert_eq!(
        ContextConfig::default().noninteractive_plan_approval,
        "approve"
    );
    assert_eq!(
        noninteractive_decision(None),
        PlanApprovalDecision::Approve,
        "missing policy must preserve the production approval default"
    );
    assert_eq!(
        noninteractive_decision(Some(" approve ")),
        PlanApprovalDecision::Approve,
        "the documented approve policy must parse after normalization"
    );
    assert_eq!(
        noninteractive_decision(Some("REJECT")),
        PlanApprovalDecision::Reject {
            reason: "noninteractive plan approval rejected by policy".into(),
        },
        "reject is the only fail-closed policy value"
    );
    for claim in [
        "`plan_model` | unset",
        "`noninteractive_plan_approval` | `\"approve\"`",
        "`\"reject\"` fails closed",
        "any other value, including the default, approves",
    ] {
        assert!(
            config.contains(claim),
            "config reference is missing policy semantic claim: {claim}"
        );
    }

    for key in ["plan_model", "noninteractive_plan_approval"] {
        assert!(
            config.contains(key),
            "config reference is missing context.{key}"
        );
    }
}
