use archon_core::skills::templates::{self, TemplateSource};
use std::path::Path;

#[test]
fn resolve_returns_embedded_when_no_overrides() {
    let tmp = tempfile::TempDir::new().unwrap();
    let (content, source) = templates::resolve_template("ai-agent-prd", tmp.path());
    assert_eq!(source, TemplateSource::Embedded);
    assert!(!content.is_empty());
}

#[test]
fn resolve_workdir_override_wins_over_embedded() {
    let tmp = tempfile::TempDir::new().unwrap();
    let templates_dir = tmp.path().join("assets/templates");
    std::fs::create_dir_all(&templates_dir).unwrap();
    std::fs::write(
        templates_dir.join("ai-agent-prd.md"),
        "Custom workdir template content",
    )
    .unwrap();
    let (content, source) = templates::resolve_template("ai-agent-prd", tmp.path());
    assert_eq!(source, TemplateSource::WorkdirOverride);
    assert_eq!(content, "Custom workdir template content");
}

#[test]
fn resolve_unknown_returns_missing() {
    let tmp = tempfile::TempDir::new().unwrap();
    let (content, source) = templates::resolve_template("nonexistent-template", tmp.path());
    assert_eq!(source, TemplateSource::Missing);
    assert!(content.is_empty());
}

#[test]
fn embedded_ai_agent_prd_nonempty() {
    assert!(templates::AI_AGENT_PRD.len() > 50_000);
}

#[test]
fn embedded_prdtospec_nonempty() {
    assert!(templates::PRD_TO_SPEC.len() > 30_000);
}
