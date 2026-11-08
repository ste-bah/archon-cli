use super::{permitted_in, project_permitted_tools};
use crate::command::workflow_decompose_prd_tools::validate_prd_launch_input;
use std::collections::BTreeSet;

const CONFIG: &str = r#"{"mcpServers":{"sample":{"toolPolicy":{"toolPermissions":{"fetch":"safe","inspect":"risky","erase":"dangerous"}}},"off":{"disabled":true,"toolPolicy":{"toolPermissions":{"fetch":"safe"}}}}}"#;
const PRD: &str = "- REQ-X-001: work\n## 1. Acceptance Criteria\n| ID | Criterion | Evidence |\n| --- | --- | --- |\n| AC-X-001 | work | observed |\n";

#[test]
fn strict_project_tools_missing_malformed_disabled_and_risk() {
    let dir = tempfile::tempdir().unwrap();
    let (path, permitted) = project_permitted_tools(dir.path()).unwrap();
    assert_eq!(path, dir.path().join(".mcp.json"));
    assert!(permitted.is_empty());
    std::fs::write(&path, "{").unwrap();
    assert!(
        project_permitted_tools(dir.path())
            .unwrap_err()
            .contains(&path.display().to_string())
    );
    std::fs::write(&path, CONFIG).unwrap();
    let (_, permitted) = project_permitted_tools(dir.path()).unwrap();
    assert_eq!(
        permitted,
        BTreeSet::from(["mcp__sample__fetch".into(), "mcp__sample__inspect".into()])
    );
    let configs = archon_mcp::config::load_config_file(&path).unwrap();
    let enabled: Vec<_> = configs
        .into_iter()
        .filter(|config| !config.disabled)
        .collect();
    assert_eq!(permitted, permitted_in(&enabled));
}

#[test]
fn intake_refuses_only_actionable_unpermitted_tokens_with_exact_joined_text() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("input.md");
    let config = dir.path().join(".mcp.json");
    std::fs::write(&config, CONFIG).unwrap();
    for suffix in [
        "\n## Required Tools\n| mcp__sample__absent | REQ-X-001 |\n| mcp__sample__erase | REQ-X-001 |\n",
        "\n- REQ-X-002: call mcp__sample__absent and mcp__sample__erase\n",
        "\n## 2. Required Tools\nmcp__sample__absent mcp__sample__erase\n",
    ] {
        std::fs::write(&path, format!("{PRD}{suffix}")).unwrap();
        let error = validate_prd_launch_input(&path, dir.path())
            .unwrap_err()
            .to_string();
        let expected = ["mcp__sample__absent", "mcp__sample__erase"]
            .map(|token| {
                archon_workflow::prd_tools::PrdToolDefect::NotPermitted {
                    token: token.into(),
                }
                .message(&path, &config)
            })
            .join(" | ");
        assert_eq!(error, expected);
        assert!(!dir.path().join(".archon/workflows").exists());
    }
}

#[test]
fn intake_accepts_missing_section_unlinked_and_example_only_tokens() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("input.md");
    for suffix in [
        "",
        "\n## Required Tools\nmcp__sample__fetch\n",
        "\n```\nmcp__sample__absent\n```\n",
        "\n```\n## Required Tools\nmcp__sample__absent\n```\n",
        "\n## Required Tools\nNone.\n",
    ] {
        std::fs::write(dir.path().join(".mcp.json"), CONFIG).unwrap();
        std::fs::write(&path, format!("{PRD}{suffix}")).unwrap();
        assert!(
            validate_prd_launch_input(&path, dir.path()).is_ok(),
            "{suffix}"
        );
    }
    std::fs::remove_file(dir.path().join(".mcp.json")).unwrap();
    std::fs::write(&path, PRD).unwrap();
    assert!(validate_prd_launch_input(&path, dir.path()).is_ok());
}

#[test]
fn r18_intake_preserves_input_and_derives_links_without_prd_edit() {
    let dir = tempfile::tempdir().unwrap();
    let prd = include_str!("../../tests/fixtures/r18_plan/prd.md");
    let path = dir.path().join("input.md");
    std::fs::write(&path, prd).unwrap();
    std::fs::write(
        dir.path().join(".mcp.json"),
        r#"{"mcpServers":{"tradingview":{"toolPolicy":{"toolPermissions":{"probe":"safe"}}}}}"#,
    )
    .unwrap();
    let (bytes, _, _) = validate_prd_launch_input(&path, dir.path()).unwrap();
    assert_eq!(bytes, prd.as_bytes());
    let (_, permitted) = project_permitted_tools(dir.path()).unwrap();
    let links = archon_workflow::prd_tools::derived_server_links(prd, &permitted);
    println!("R18 derived links: {links:?}");
    assert_eq!(
        links["tradingview"],
        BTreeSet::from([
            "AC-DL-007".into(),
            "G-DL-002".into(),
            "RD-004".into(),
            "REQ-DL-033".into(),
        ])
    );
    assert!(!dir.path().join(".archon/workflows").exists());
}
