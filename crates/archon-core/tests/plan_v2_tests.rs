use archon_core::plan_explore::{ExploreConfig, create_explore_request};
use archon_core::plan_v2::{Plan, PlanStatus, StepStatus};

// ── Plan CRUD ──────────────────────────────────────────────────────

#[test]
fn new_plan_has_draft_status() {
    let plan = Plan::new("Implement auth");
    assert_eq!(plan.status, PlanStatus::Draft);
    assert_eq!(plan.title, "Implement auth");
    assert!(plan.steps.is_empty());
}

#[test]
fn add_step_increments_index() {
    let mut plan = Plan::new("Test plan");
    plan.add_step("First step", vec!["a.rs".into()], "low");
    plan.add_step("Second step", vec!["b.rs".into()], "medium");
    plan.add_step("Third step", vec!["c.rs".into()], "high");

    assert_eq!(plan.steps.len(), 3);
    assert_eq!(plan.steps[0].index, 1);
    assert_eq!(plan.steps[1].index, 2);
    assert_eq!(plan.steps[2].index, 3);
}

#[test]
fn mark_step_in_progress() {
    let mut plan = Plan::new("Test plan");
    plan.add_step("Do something", vec![], "low");
    assert_eq!(plan.steps[0].status, StepStatus::Pending);

    plan.mark_step_in_progress(1).expect("should succeed");
    assert_eq!(plan.steps[0].status, StepStatus::InProgress);
}

#[test]
fn mark_step_complete() {
    let mut plan = Plan::new("Test plan");
    plan.add_step("Do something", vec![], "low");
    plan.mark_step_in_progress(1).expect("should succeed");
    plan.mark_step_complete(1).expect("should succeed");
    assert_eq!(plan.steps[0].status, StepStatus::Complete);
}

#[test]
fn mark_step_skipped() {
    let mut plan = Plan::new("Test plan");
    plan.add_step("Do something", vec![], "low");
    plan.mark_step_skipped(1).expect("should succeed");
    assert_eq!(plan.steps[0].status, StepStatus::Skipped);
}

#[test]
fn invalid_index_errors() {
    let mut plan = Plan::new("Test plan");
    plan.add_step("Only step", vec![], "low");

    assert!(plan.mark_step_in_progress(99).is_err());
    assert!(plan.mark_step_complete(0).is_err());
    assert!(plan.mark_step_skipped(2).is_err());
}

#[test]
fn overall_progress() {
    let mut plan = Plan::new("Test plan");
    for i in 0..5 {
        plan.add_step(&format!("Step {}", i + 1), vec![], "low");
    }
    plan.mark_step_in_progress(1).expect("ok");
    plan.mark_step_complete(1).expect("ok");
    plan.mark_step_in_progress(2).expect("ok");
    plan.mark_step_complete(2).expect("ok");

    let (completed, total) = plan.overall_progress();
    assert_eq!(completed, 2);
    assert_eq!(total, 5);
}

// ── Display ────────────────────────────────────────────────────────

#[test]
fn to_display_shows_steps() {
    let mut plan = Plan::new("Auth feature");
    plan.add_step("Create model", vec!["model.rs".into()], "low");
    plan.add_step("Add routes", vec!["routes.rs".into()], "medium");

    let display = plan.to_display();
    assert!(
        display.contains("Create model"),
        "display should contain step description"
    );
    assert!(
        display.contains("Add routes"),
        "display should contain step description"
    );
}

#[test]
fn to_display_shows_status() {
    let mut plan = Plan::new("Auth feature");
    plan.add_step("Pending step", vec![], "low");
    plan.add_step("Done step", vec![], "low");
    plan.mark_step_in_progress(2).expect("ok");
    plan.mark_step_complete(2).expect("ok");

    let display = plan.to_display();
    // Should have some status indicator for pending vs complete
    assert!(display.contains("Pending step"));
    assert!(display.contains("Done step"));
    // Check for status indicators (exact format tested by presence of distinct markers)
    assert!(
        display.contains("[ ]") || display.contains("pending") || display.contains("\u{25cb}"),
        "display should show pending indicator"
    );
    assert!(
        display.contains("[x]") || display.contains("complete") || display.contains("\u{2713}"),
        "display should show complete indicator"
    );
}

#[test]
fn to_prompt_block_format() {
    let plan = Plan::new("Auth feature");
    let block = plan.to_prompt_block();
    assert!(
        block.contains("Plan:"),
        "prompt block should contain 'Plan:' header"
    );
}

// ── Persistence ────────────────────────────────────────────────────

#[test]
fn save_and_load_plan() {
    let tmp = tempfile::TempDir::new().expect("tmpdir");
    let db_path = tmp.path().join("test.db");
    let store = archon_session::storage::SessionStore::open(&db_path).expect("open store");

    let session_id = "test-session-001";
    store
        .register_session(session_id, "/tmp", None, "test-model")
        .expect("register session");

    let mut plan = Plan::new("Persistence test");
    plan.add_step("Step one", vec!["file.rs".into()], "medium");
    plan.add_step("Step two", vec![], "low");
    plan.mark_step_in_progress(1).expect("ok");
    plan.mark_step_complete(1).expect("ok");

    archon_core::plan_v2::save_plan(&store, session_id, &plan).expect("save");
    let loaded = archon_core::plan_v2::load_plan(&store, session_id)
        .expect("load")
        .expect("should be Some");

    assert_eq!(loaded.id, plan.id);
    assert_eq!(loaded.title, "Persistence test");
    assert_eq!(loaded.steps.len(), 2);
    assert_eq!(loaded.steps[0].status, StepStatus::Complete);
    assert_eq!(loaded.steps[1].status, StepStatus::Pending);
    assert_eq!(loaded.status, plan.status);
}

#[test]
fn load_nonexistent_returns_none() {
    let tmp = tempfile::TempDir::new().expect("tmpdir");
    let db_path = tmp.path().join("test2.db");
    let store = archon_session::storage::SessionStore::open(&db_path).expect("open store");

    let session_id = "no-plan-session";
    store
        .register_session(session_id, "/tmp", None, "test-model")
        .expect("register session");

    let result = archon_core::plan_v2::load_plan(&store, session_id).expect("load");
    assert!(result.is_none());
}

// ── File tracking ──────────────────────────────────────────────────

#[test]
fn on_file_edited_marks_step() {
    let mut plan = Plan::new("File tracking");
    plan.add_step(
        "Update auth",
        vec!["auth.rs".into(), "config.rs".into()],
        "medium",
    );
    plan.add_step("Update tests", vec!["test.rs".into()], "low");

    plan.on_file_edited("auth.rs");
    assert_eq!(plan.steps[0].status, StepStatus::InProgress);
    assert_eq!(plan.steps[1].status, StepStatus::Pending);
}

#[test]
fn on_file_edited_no_match() {
    let mut plan = Plan::new("File tracking");
    plan.add_step("Update auth", vec!["auth.rs".into()], "medium");

    plan.on_file_edited("unrelated.rs");
    assert_eq!(plan.steps[0].status, StepStatus::Pending);
}

// ── Explore ────────────────────────────────────────────────────────

#[test]
fn explore_config_defaults() {
    let config = ExploreConfig::default();
    assert!(config.allowed_tools.contains(&"Read".to_string()));
    assert!(config.allowed_tools.contains(&"Glob".to_string()));
    assert!(config.allowed_tools.contains(&"Grep".to_string()));
    assert!(config.allowed_tools.contains(&"ToolSearch".to_string()));
    assert_eq!(config.max_turns, 5);
}

#[test]
fn explore_config_no_write_tools() {
    let config = ExploreConfig::default();
    assert!(!config.allowed_tools.contains(&"Write".to_string()));
    assert!(!config.allowed_tools.contains(&"Edit".to_string()));
    assert!(!config.allowed_tools.contains(&"Bash".to_string()));
}

#[test]
fn create_explore_request_returns_subagent_request() {
    let request = create_explore_request("Find all auth modules");
    assert!(request.prompt.contains("Find all auth modules"));
    assert_eq!(
        request.allowed_tools,
        ExploreConfig::default().allowed_tools
    );
    assert_eq!(request.max_turns, ExploreConfig::default().max_turns);
    assert!(request.model.is_none(), "explore should use parent model");
}

#[test]
fn plan_explore_step_returns_subagent_request() {
    let mut plan = Plan::new("Test plan");
    plan.add_step(
        "Implement auth module",
        vec!["src/auth.rs".into()],
        "medium",
    );
    plan.add_step("Add tests", vec!["tests/auth_test.rs".into()], "low");

    let request = plan.explore_step(1).expect("step 1 should exist");
    assert!(request.prompt.contains("Implement auth module"));
    assert!(request.prompt.contains("src/auth.rs"));
    assert!(request.allowed_tools.contains(&"Read".to_string()));
    assert!(!request.allowed_tools.contains(&"Write".to_string()));
}

#[test]
fn plan_explore_step_invalid_index() {
    let plan = Plan::new("Empty plan");
    assert!(plan.explore_step(99).is_err());
}
