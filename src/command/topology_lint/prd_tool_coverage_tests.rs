use super::*;
use archon_core::config::GateMode;
use archon_workflow::RemediationScope;
use std::collections::BTreeSet;

struct Fixture {
    dir: tempfile::TempDir,
    root: PathBuf,
}
impl Fixture {
    fn new(prd: &str) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("tasks/PRD-EXAMPLE");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(dir.path().join("tasks/PRD-EXAMPLE.md"), prd).unwrap();
        std::fs::write(dir.path().join(".mcp.json"), r#"{"mcpServers":{"sample":{"toolPolicy":{"toolPermissions":{"fetch":"safe","inspect":"safe"}}}}}"#).unwrap();
        Self { dir, root }
    }
    fn task(&self, id: &str, implements: &[&str], tools: &[&str]) -> PathBuf {
        let path = self.root.join(format!("{id}.md"));
        let raw = format!(
            "```yaml\ntask_id: {id}\ntitle: work\ncomplexity: low\nstatus: ready\ndepends_on: []\nblocks: []\nimplements: {}\nrequired_tools: {}\nrequired_env_keys: []\ndeliverable_contracts: []\nsource_sections: ['1']\n```\n\n## Focused Tests\n- `echo verified`\n",
            serde_json::to_string(implements).unwrap(),
            serde_json::to_string(tools).unwrap()
        );
        std::fs::write(&path, raw).unwrap();
        path
    }
    fn evaluate(&self) -> crate::command::workflow_gate::GateEvaluation {
        evaluate_lint(
            self.dir.path(),
            &LintSource::Tasks(self.root.clone()),
            GateMode::Enforce,
        )
        .unwrap()
    }
    fn tool_findings(&self) -> Vec<crate::command::workflow_gate::GateFinding> {
        self.evaluate()
            .findings
            .into_iter()
            .filter(|finding| {
                finding
                    .deterministic_defect
                    .as_ref()
                    .is_some_and(|identity| identity.code.starts_with("prd_tool"))
            })
            .collect()
    }
    fn draft(&self, mut skeleton: serde_json::Value) {
        use archon_workflow::task_set_contract::{
            ACCEPTANCE_CONTRACT_FILE, ACCEPTANCE_LOCK_FILE, TASK_SKELETON_FILE, content_digest,
            empty_gate_findings_digest,
        };
        let prd_path = self.dir.path().join("tasks/PRD-EXAMPLE.md");
        let mut prd = std::fs::read_to_string(&prd_path).unwrap();
        prd.push_str(
            "\n## Acceptance Criteria\n| ID | Criterion |\n| --- | --- |\n| AC-X-001 | work |\n",
        );
        std::fs::write(&prd_path, &prd).unwrap();
        let contract = serde_json::json!({
            "schema_version":1,"prd":{"path":"tasks/PRD-EXAMPLE.md","digest":content_digest(prd.as_bytes())},
            "gap_policy":{},"supplementary":[],
            "acceptance":[{"id":"AC-X-001","criterion":"work","check":{"kind":"command","command":"echo work","cwd":"project_root"},"gap_permitted":false,"judgment":{"verdict":"accepted","counterexample":"attempted","reason":"observed","host_call_id":"test"}}]
        });
        let bytes = serde_json::to_vec(&contract).unwrap();
        let digest = content_digest(&bytes);
        let stamp = serde_json::json!({"mode":"enforce","finding_count":0,"findings_digest":empty_gate_findings_digest(),"binary_commit":"test","evaluated_at":"2026-11-08T00:00:00Z"});
        std::fs::write(self.root.join(ACCEPTANCE_CONTRACT_FILE), bytes).unwrap();
        std::fs::write(
            self.root.join(ACCEPTANCE_LOCK_FILE),
            serde_json::to_vec(
                &serde_json::json!({"algorithm":"blake3","digest":digest,"gate":stamp}),
            )
            .unwrap(),
        )
        .unwrap();
        let pin =
            crate::command::workflow_task_set::acceptance_pin_path(self.dir.path(), &self.root);
        std::fs::create_dir_all(pin.parent().unwrap()).unwrap();
        std::fs::write(pin, serde_json::to_vec(&serde_json::json!({"task_root":archon_shell::paths::plain(self.root.canonicalize().unwrap()),"acceptance_digest":digest,"freeze_event_id":"test","acceptance_gate":stamp})).unwrap()).unwrap();
        skeleton["acceptance_digest"] = serde_json::json!(digest);
        std::fs::write(
            self.root.join(TASK_SKELETON_FILE),
            serde_json::to_vec(&skeleton).unwrap(),
        )
        .unwrap();
    }
}
const DECLARED: &str = "- REQ-X-001: work\n## 1. Required Tools\n| Tool | Used by | Purpose |\n| --- | --- | --- |\n| mcp__sample__fetch | REQ-X-001 | work |\n";

#[test]
fn union_is_satisfied_anywhere_by_bare_qualified_or_action_name() {
    for entry in ["fetch", "mcp__sample__fetch", "mcp_action:fetch"] {
        let fixture = Fixture::new(DECLARED);
        fixture.task("TASK-X-001", &["REQ-X-001"], &[]);
        fixture.task("TASK-X-002", &[], &[entry]);
        assert!(fixture.tool_findings().is_empty(), "{entry}");
    }
}

#[test]
fn uncovered_token_has_body_author_path_subject_and_routing_text() {
    let fixture = Fixture::new(DECLARED);
    let path = fixture.task("TASK-X-001", &["REQ-X-001"], &[]);
    let findings = fixture.tool_findings();
    assert_eq!(findings.len(), 1);
    let finding = &findings[0];
    assert_eq!(finding.subject, "TASK-X-001");
    assert_eq!(finding.source_path.as_deref(), Some(path.as_path()));
    assert_eq!(finding.remediation_scope, RemediationScope::Body);
    assert!(
        regex::Regex::new(r"\btask\s+TASK-X-001:")
            .unwrap()
            .is_match(&finding.text)
    );
    assert_eq!(
        finding.deterministic_defect.as_ref().unwrap().location,
        "required_tools"
    );
}

#[test]
fn skeleton_order_selects_owner_and_only_names_owned_links() {
    let fixture = Fixture::new(
        &DECLARED
            .replace("| REQ-X-001 | work |", "| REQ-X-001, REQ-X-002 | work |")
            .replace("## 1.", "- REQ-X-002: other\n## 1."),
    );
    fixture.task("TASK-X-001", &["REQ-X-001"], &[]);
    fixture.task("TASK-X-002", &["REQ-X-002"], &[]);
    let skeleton = serde_json::json!({"schema_version":1,"acceptance_digest":"", "tasks":[
        {"task_id":"TASK-X-002","file_name":"TASK-X-002.md","implements":["REQ-X-002"],"source_sections":["1"]},
        {"task_id":"TASK-X-001","file_name":"TASK-X-001.md","implements":["REQ-X-001"],"source_sections":["1"]}
    ]});
    fixture.draft(skeleton);
    let findings = fixture.tool_findings();
    assert_eq!(findings[0].subject, "TASK-X-002");
    assert!(
        findings[0]
            .text
            .contains("REQ-X-002 that this task implements")
    );
    assert!(!findings[0].text.contains("REQ-X-001"));
}

#[test]
fn no_owner_routes_to_skeleton() {
    let fixture = Fixture::new(DECLARED);
    fixture.task("TASK-X-001", &[], &[]);
    let findings = fixture.tool_findings();
    assert_eq!(findings[0].subject, "REQ-X-001");
    assert_eq!(findings[0].remediation_scope, RemediationScope::Skeleton);
    assert_eq!(
        findings[0]
            .source_path
            .as_ref()
            .unwrap()
            .file_name()
            .unwrap(),
        "task-skeleton.json"
    );
}

#[test]
fn broken_task_suppresses_union_and_preflight_reports_parse_error() {
    let fixture = Fixture::new(DECLARED);
    fixture.task("TASK-X-001", &["REQ-X-001"], &[]);
    std::fs::write(fixture.root.join("TASK-X-002.md"), "invalid").unwrap();
    assert!(
        prd_tool_coverage::set_findings(fixture.dir.path(), &fixture.root)
            .unwrap()
            .is_empty()
    );
    assert!(
        evaluate_lint(
            fixture.dir.path(),
            &LintSource::Tasks(fixture.root.clone()),
            GateMode::Enforce
        )
        .is_err()
    );
}

#[test]
fn malformed_project_config_is_operational_and_never_empty_permission_verdict() {
    let fixture = Fixture::new(DECLARED);
    fixture.task("TASK-X-001", &["REQ-X-001"], &[]);
    std::fs::write(fixture.dir.path().join(".mcp.json"), "{").unwrap();
    let evaluation = fixture.evaluate();
    assert!(
        evaluation
            .operational_error()
            .unwrap()
            .contains("prd tool coverage failed:")
    );
    assert!(
        evaluation
            .operational_error()
            .unwrap()
            .contains(".mcp.json")
    );
    assert!(!evaluation.findings.iter().any(|finding| {
        finding
            .deterministic_defect
            .as_ref()
            .is_some_and(|identity| identity.code == "prd_tool_not_permitted")
    }));
}

#[test]
fn only_actionable_unpermitted_tools_are_prd_input() {
    for (prd, expected) in [
        (
            DECLARED.replace("mcp__sample__fetch", "mcp__sample__absent"),
            1,
        ),
        ("- REQ-X-001: call mcp__sample__absent\n".into(), 1),
        (
            "- REQ-X-001: work\n```\nmcp__sample__absent\n```\n".into(),
            0,
        ),
    ] {
        let fixture = Fixture::new(&prd);
        fixture.task("TASK-X-001", &["REQ-X-001"], &["fetch"]);
        let findings = fixture.tool_findings();
        assert_eq!(findings.len(), expected, "{findings:?}");
        assert!(
            findings
                .iter()
                .all(|finding| finding.remediation_scope == RemediationScope::PrdInput)
        );
    }
}

#[test]
fn missing_section_and_unlinked_section_derive_server_links() {
    for prd in [
        "- REQ-X-001: use sample MCP\n",
        "- REQ-X-001: use sample MCP\n## 1. Required Tools\nmcp__sample__fetch\n",
        "- REQ-X-001: use sample MCP\n## 1. Required Tools\nmcp__sample__inspect\n",
    ] {
        let fixture = Fixture::new(prd);
        fixture.task("TASK-X-001", &["REQ-X-001"], &[]);
        let findings = fixture.tool_findings();
        assert!(!findings.is_empty());
        assert!(
            findings
                .iter()
                .all(|finding| finding.remediation_scope == RemediationScope::Body)
        );
        fixture.task("TASK-X-001", &["REQ-X-001"], &["fetch", "inspect"]);
        assert!(fixture.tool_findings().is_empty());
    }
}

#[test]
fn missing_section_without_server_and_unlinked_without_derived_link_only_diagnose() {
    for prd in [
        "- REQ-X-001: work\n",
        "- REQ-X-001: work\n## 1. Required Tools\nmcp__sample__fetch\n",
        "- REQ-X-001: work\n## 1. Required Tools\nNone.\n",
    ] {
        let fixture = Fixture::new(prd);
        fixture.task("TASK-X-001", &["REQ-X-001"], &[]);
        assert!(fixture.tool_findings().is_empty());
    }
}

#[test]
fn r18_preexisting_skeleton_derives_union_findings_without_prd_input() {
    let fixture = Fixture::new(include_str!("../../../tests/fixtures/r18_plan/prd.md"));
    std::fs::write(
        fixture.dir.path().join(".mcp.json"),
        r#"{"mcpServers":{"tradingview":{"toolPolicy":{"toolPermissions":{"probe":"safe"}}}}}"#,
    )
    .unwrap();
    let skeleton: archon_workflow::task_skeleton::TaskSkeleton = serde_json::from_str(
        include_str!("../../../tests/fixtures/r18_plan/task-skeleton.json"),
    )
    .unwrap();
    for task in &skeleton.tasks {
        let ids: Vec<_> = task.implements.iter().map(String::as_str).collect();
        fixture.task(&task.task_id, &ids, &[]);
    }
    // The existing skeleton supplies ownership order; this check never edits it.
    std::fs::write(
        fixture.root.join("task-skeleton.json"),
        serde_json::to_vec(&skeleton).unwrap(),
    )
    .unwrap();
    let findings = prd_tool_coverage::set_findings(fixture.dir.path(), &fixture.root).unwrap();
    assert!(!findings.is_empty());
    assert!(findings.iter().all(|finding| matches!(
        finding.remediation_scope,
        RemediationScope::Body | RemediationScope::Skeleton
    )));
    let subjects: BTreeSet<_> = findings
        .iter()
        .map(|finding| finding.subject.clone())
        .collect();
    println!("R18 derived tool owner subjects: {subjects:?}");
    assert_eq!(findings.len(), 1);
    assert_eq!(subjects, BTreeSet::from(["TASK-TDL-005".to_owned()]));
    let task = skeleton
        .tasks
        .iter()
        .find(|task| task.task_id == "TASK-TDL-005")
        .unwrap();
    let ids: Vec<_> = task.implements.iter().map(String::as_str).collect();
    fixture.task(&task.task_id, &ids, &["probe"]);
    assert!(
        prd_tool_coverage::set_findings(fixture.dir.path(), &fixture.root)
            .unwrap()
            .is_empty()
    );
    std::fs::write(
        fixture.dir.path().join(".mcp.json"),
        r#"{"mcpServers":{"TradingView":{"toolPolicy":{"toolPermissions":{"probe":"safe"}}}}}"#,
    )
    .unwrap();
    assert!(
        prd_tool_coverage::set_findings(fixture.dir.path(), &fixture.root)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn exact_id_text_token_keeps_union_semantics_without_a_section() {
    let fixture = Fixture::new("- REQ-X-001: call mcp__sample__fetch\n");
    fixture.task("TASK-X-001", &["REQ-X-001"], &[]);
    fixture.task("TASK-X-002", &[], &["fetch"]);
    assert!(fixture.tool_findings().is_empty());
}

#[test]
fn derived_obligation_without_owner_routes_to_skeleton_and_lists_permitted_names() {
    let fixture = Fixture::new("- REQ-X-001: call sample server\n");
    fixture.task("TASK-X-001", &[], &[]);
    let findings = fixture.tool_findings();
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].remediation_scope, RemediationScope::Skeleton);
    for token in ["mcp__sample__fetch", "mcp__sample__inspect"] {
        assert!(findings[0].text.contains(token));
    }
}

#[test]
fn derived_server_union_is_covered_by_any_link_owner_and_finds_once() {
    let fixture = Fixture::new(
        "- REQ-X-001: use sample server\n- REQ-X-002: use sample MCP\n- REQ-X-003: call `sample`\n",
    );
    fixture.task("TASK-X-001", &["REQ-X-001"], &[]);
    fixture.task("TASK-X-002", &["REQ-X-002"], &[]);
    let findings = fixture.tool_findings();
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].subject, "TASK-X-001");
    fixture.task("TASK-X-003", &[], &["fetch"]);
    assert_eq!(fixture.tool_findings().len(), 1);
    for tool in ["fetch", "mcp__sample__inspect", "mcp_action:fetch"] {
        fixture.task("TASK-X-002", &["REQ-X-002"], &[tool]);
        assert!(fixture.tool_findings().is_empty());
    }
}

#[test]
fn explicit_token_owner_also_covers_other_derived_links_of_its_server() {
    let fixture = Fixture::new(
        "- REQ-X-001: use sample server via mcp__sample__fetch\n- REQ-X-002: use sample server\n",
    );
    fixture.task("TASK-X-001", &["REQ-X-001"], &["fetch"]);
    fixture.task("TASK-X-002", &["REQ-X-002"], &[]);
    assert!(fixture.tool_findings().is_empty());
}

#[test]
fn derived_owner_uses_most_links_then_skeleton_order() {
    let fixture = Fixture::new(
        "- REQ-X-001: use sample server\n- REQ-X-002: use sample server\n- REQ-X-003: use sample server\n",
    );
    fixture.task("TASK-X-001", &["REQ-X-001"], &[]);
    fixture.task("TASK-X-002", &["REQ-X-002", "REQ-X-003"], &[]);
    assert_eq!(fixture.tool_findings()[0].subject, "TASK-X-002");
    fixture.task("TASK-X-001", &["REQ-X-001", "REQ-X-002"], &[]);
    fixture.draft(serde_json::json!({"schema_version":1,"acceptance_digest":"", "tasks":[
        {"task_id":"TASK-X-002","file_name":"TASK-X-002.md","implements":["REQ-X-002", "REQ-X-003"],"source_sections":["1"]},
        {"task_id":"TASK-X-001","file_name":"TASK-X-001.md","implements":["REQ-X-001", "REQ-X-002"],"source_sections":["1"]}
    ]}));
    assert_eq!(fixture.tool_findings()[0].subject, "TASK-X-002");
}

#[test]
fn same_owner_findings_merge_identity_and_retain_every_tool_demand() {
    let fixture = Fixture::new(
        "- REQ-X-001: use sample server via mcp__sample__fetch and mcp__sample__inspect\n",
    );
    fixture.task("TASK-X-001", &["REQ-X-001"], &[]);
    let findings = fixture.tool_findings();
    assert_eq!(findings.len(), 1);
    for name in ["mcp__sample__fetch", "mcp__sample__inspect"] {
        assert!(findings[0].text.contains(&format!("add exactly '{name}'")));
    }
    assert!(findings[0].text.contains("configured server"));
    fixture.task("TASK-X-001", &["REQ-X-001"], &["fetch"]);
    let findings = fixture.tool_findings();
    assert_eq!(findings.len(), 1);
    assert!(
        findings[0]
            .text
            .contains("add exactly 'mcp__sample__inspect'")
    );
}

#[test]
fn refusal_finding_text_uses_project_relative_paths() {
    let fixture = Fixture::new(&DECLARED.replace("mcp__sample__fetch", "mcp__sample__absent"));
    fixture.task("TASK-X-001", &["REQ-X-001"], &[]);
    let findings = fixture.tool_findings();
    assert_eq!(findings.len(), 1);
    assert!(
        !findings[0]
            .text
            .contains(&fixture.dir.path().display().to_string())
    );
    assert!(findings[0].text.contains("PRD tasks"));
    assert!(findings[0].text.contains("config .mcp.json"));
}
