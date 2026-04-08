//! Integration tests for FinalStageOrchestrator (Phase 8 final assembly).
//!
//! Tests TASK-PIPE-C06: research::final_stage module.

use archon_pipeline::research::final_stage::combiner::{ChapterContent, combine_chapters};
use archon_pipeline::research::final_stage::mapper::map_to_chapters;
use archon_pipeline::research::final_stage::scanner::{AgentOutput, scan_outputs};
use archon_pipeline::research::final_stage::{
    FinalStageError, FinalStageOptions, FinalStageOrchestrator, FinalStageState,
};
use std::fs;
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Test data helpers
// ---------------------------------------------------------------------------

/// Create a temporary directory populated with numbered agent output files.
fn create_agent_output_dir(files: &[(&str, &str)]) -> TempDir {
    let dir = TempDir::new().expect("should create temp dir");
    for (name, content) in files {
        let path = dir.path().join(name);
        fs::write(&path, content).expect("should write test file");
    }
    dir
}

/// Returns a standard set of agent output files for testing.
fn standard_agent_files() -> Vec<(&'static str, &'static str)> {
    vec![
        (
            "01-topic-researcher.md",
            "# Topic Research\n\nResearch on quantum computing fundamentals.",
        ),
        (
            "02-literature-scanner.md",
            "# Literature Scan\n\nKey papers identified in the field.",
        ),
        (
            "03-ambiguity-clarifier.md",
            "# Ambiguity Clarification\n\nResolved unclear research questions.",
        ),
        (
            "04-methodology-designer.md",
            "# Methodology Design\n\nExperimental approach defined.",
        ),
        (
            "05-data-analyst.md",
            "# Data Analysis\n\nStatistical methods and results.",
        ),
    ]
}

/// Returns a set of pre-built chapter contents for combiner tests.
fn sample_chapters() -> Vec<ChapterContent> {
    vec![
        ChapterContent {
            number: 1,
            title: "Introduction".to_string(),
            content: "This chapter introduces the research topic.\n\nQuantum computing is a rapidly evolving field.".to_string(),
        },
        ChapterContent {
            number: 2,
            title: "Literature Review".to_string(),
            content: "This chapter reviews existing literature.\n\nPrior work has established foundational principles.".to_string(),
        },
        ChapterContent {
            number: 3,
            title: "Methodology".to_string(),
            content: "This chapter describes the methodology.\n\nWe employ a mixed-methods approach.".to_string(),
        },
    ]
}

/// Returns a simple chapter list for mapper tests.
fn sample_chapter_list() -> Vec<(u32, String)> {
    vec![
        (1, "Introduction".to_string()),
        (2, "Literature Review".to_string()),
        (3, "Methodology".to_string()),
        (4, "Results".to_string()),
        (5, "Conclusion".to_string()),
    ]
}

/// Helper to build default FinalStageOptions.
fn default_options() -> FinalStageOptions {
    FinalStageOptions {
        force: false,
        dry_run: false,
        style_profile_id: None,
        token_budget: None,
    }
}

// ---------------------------------------------------------------------------
// State machine tests
// ---------------------------------------------------------------------------

#[test]
fn test_valid_state_transitions() {
    let valid_chain = [
        FinalStageState::Idle,
        FinalStageState::Initializing,
        FinalStageState::Scanning,
        FinalStageState::Summarizing,
        FinalStageState::Mapping,
        FinalStageState::Writing,
        FinalStageState::Combining,
        FinalStageState::Validating,
        FinalStageState::Completed,
    ];

    for window in valid_chain.windows(2) {
        let from = window[0];
        let to = window[1];
        let result = FinalStageOrchestrator::validate_transition(from, to);
        assert!(
            result.is_ok(),
            "transition {:?} -> {:?} should be valid, got: {:?}",
            from,
            to,
            result,
        );
    }
}

#[test]
fn test_invalid_state_transition() {
    let result = FinalStageOrchestrator::validate_transition(
        FinalStageState::Idle,
        FinalStageState::Completed,
    );
    assert!(result.is_err(), "Idle -> Completed should be invalid");

    match result.unwrap_err() {
        FinalStageError::InvalidTransition { from, to } => {
            assert_eq!(from, FinalStageState::Idle);
            assert_eq!(to, FinalStageState::Completed);
        }
        other => panic!("expected InvalidTransition, got: {:?}", other),
    }
}

#[test]
fn test_any_state_can_transition_to_failed() {
    let all_states = [
        FinalStageState::Idle,
        FinalStageState::Initializing,
        FinalStageState::Scanning,
        FinalStageState::Summarizing,
        FinalStageState::Mapping,
        FinalStageState::Writing,
        FinalStageState::Combining,
        FinalStageState::Validating,
        FinalStageState::Completed,
    ];

    for state in &all_states {
        let result = FinalStageOrchestrator::validate_transition(*state, FinalStageState::Failed);
        assert!(
            result.is_ok(),
            "transition {:?} -> Failed should be valid",
            state,
        );
    }
}

#[test]
fn test_initial_state_is_idle() {
    let orchestrator = FinalStageOrchestrator::new();
    assert_eq!(
        orchestrator.state(),
        FinalStageState::Idle,
        "new orchestrator must start in Idle state"
    );
}

// ---------------------------------------------------------------------------
// Scanner tests
// ---------------------------------------------------------------------------

#[test]
fn test_scanner_parses_agent_files() {
    let files = standard_agent_files();
    let dir = create_agent_output_dir(&files);

    let outputs = scan_outputs(dir.path()).expect("should scan outputs");

    assert_eq!(outputs.len(), 5, "should find 5 markdown files");

    // Check that the first file is parsed correctly.
    let first = outputs
        .iter()
        .find(|o| o.agent_key == "topic-researcher")
        .expect("should find topic-researcher output");
    assert_eq!(first.phase, 1, "index 01 should map to phase 1");
    assert!(
        first.content.contains("quantum computing"),
        "content should be read from file"
    );
    assert!(
        first.file_path.ends_with("01-topic-researcher.md"),
        "file_path should preserve original filename"
    );

    // Check ambiguity-clarifier (index 03).
    let third = outputs
        .iter()
        .find(|o| o.agent_key == "ambiguity-clarifier")
        .expect("should find ambiguity-clarifier output");
    assert_eq!(third.phase, 3);
}

#[test]
fn test_scanner_handles_empty_directory() {
    let dir = TempDir::new().expect("should create temp dir");
    let outputs = scan_outputs(dir.path()).expect("should handle empty dir");
    assert!(
        outputs.is_empty(),
        "empty directory should yield empty Vec<AgentOutput>"
    );
}

#[test]
fn test_scanner_ignores_non_md_files() {
    let dir = TempDir::new().expect("should create temp dir");
    fs::write(dir.path().join("01-agent-a.md"), "valid markdown").unwrap();
    fs::write(dir.path().join("02-agent-b.txt"), "text file ignored").unwrap();
    fs::write(dir.path().join("notes.json"), "{}").unwrap();

    let outputs = scan_outputs(dir.path()).expect("should scan outputs");
    assert_eq!(outputs.len(), 1, "only .md files should be scanned");
    assert_eq!(outputs[0].agent_key, "agent-a");
}

// ---------------------------------------------------------------------------
// Mapper tests
// ---------------------------------------------------------------------------

#[test]
fn test_mapper_assigns_outputs_to_chapters() {
    let agent_outputs = vec![
        AgentOutput {
            agent_key: "methodology-designer".to_string(),
            phase: 4,
            content: "Experimental design for the study.".to_string(),
            file_path: "04-methodology-designer.md".to_string(),
        },
        AgentOutput {
            agent_key: "literature-scanner".to_string(),
            phase: 2,
            content: "Review of prior work in the field.".to_string(),
            file_path: "02-literature-scanner.md".to_string(),
        },
    ];

    let chapters = sample_chapter_list();
    let mapping = map_to_chapters(agent_outputs, &chapters);

    // Methodology agent should map to the Methodology chapter (number 3).
    let methodology_outputs = mapping.mappings.get(&3);
    assert!(
        methodology_outputs.is_some(),
        "methodology chapter should have mapped outputs"
    );
    let meth = methodology_outputs.unwrap();
    assert!(
        meth.iter().any(|m| m.agent_key == "methodology-designer"),
        "methodology-designer should map to methodology chapter"
    );

    // Literature agent should map to Literature Review chapter (number 2).
    let lit_outputs = mapping.mappings.get(&2);
    assert!(
        lit_outputs.is_some(),
        "literature review chapter should have mapped outputs"
    );
    let lit = lit_outputs.unwrap();
    assert!(
        lit.iter().any(|m| m.agent_key == "literature-scanner"),
        "literature-scanner should map to literature review chapter"
    );
}

#[test]
fn test_mapper_heuristic_fallback() {
    // Agent outputs with keys that don't semantically match any chapter.
    let agent_outputs = vec![
        AgentOutput {
            agent_key: "generic-agent-001".to_string(),
            phase: 1,
            content: "Some generic research content.".to_string(),
            file_path: "01-generic-agent-001.md".to_string(),
        },
        AgentOutput {
            agent_key: "generic-agent-002".to_string(),
            phase: 2,
            content: "More generic research content.".to_string(),
            file_path: "02-generic-agent-002.md".to_string(),
        },
    ];

    let chapters = sample_chapter_list();
    let mapping = map_to_chapters(agent_outputs, &chapters);

    // Heuristic fallback should distribute outputs across chapters (no chapter empty
    // or all outputs lost). At minimum, the total mapped outputs should equal input count.
    let total_mapped: usize = mapping.mappings.values().map(|v| v.len()).sum();
    assert!(
        total_mapped >= 2,
        "heuristic fallback should map all outputs; got {} mapped",
        total_mapped,
    );
}

// ---------------------------------------------------------------------------
// Writer / EC-PIPE-007 tests
// ---------------------------------------------------------------------------

#[test]
fn test_chapter_writer_placeholder_on_failure() {
    let chapter_title = "Methodology";
    let error_msg = "LLM API returned 503 Service Unavailable";

    let placeholder =
        FinalStageOrchestrator::generate_chapter_placeholder(chapter_title, error_msg);

    assert!(
        placeholder.contains("## Methodology"),
        "placeholder should contain chapter heading, got: {}",
        placeholder,
    );
    assert!(
        placeholder.contains("could not be generated"),
        "placeholder should indicate generation failure"
    );
    assert!(
        placeholder.contains(error_msg),
        "placeholder should include the error description"
    );
}

#[test]
fn test_placeholder_format() {
    let placeholder =
        FinalStageOrchestrator::generate_chapter_placeholder("Results", "timeout after 30s");

    // Must start with an H2 heading containing the chapter title.
    assert!(
        placeholder.starts_with("## Results"),
        "placeholder must start with '## Results', got: {}",
        placeholder,
    );

    // Must contain the error in italics.
    assert!(
        placeholder.contains("*") && placeholder.contains("timeout after 30s"),
        "placeholder should contain the error in italicized text"
    );
}

// ---------------------------------------------------------------------------
// Combiner tests
// ---------------------------------------------------------------------------

#[test]
fn test_combiner_produces_final_paper() {
    let chapters = sample_chapters();
    let paper = combine_chapters(&chapters);

    // Every chapter's content should appear in the combined paper.
    for ch in &chapters {
        assert!(
            paper.contains(&ch.content),
            "final paper should contain chapter '{}' content",
            ch.title,
        );
    }
}

#[test]
fn test_combiner_toc_generation() {
    let chapters = sample_chapters();
    let paper = combine_chapters(&chapters);

    assert!(
        paper.contains("Table of Contents") || paper.contains("table of contents"),
        "final paper should include a Table of Contents section"
    );

    // ToC should reference each chapter by title.
    for ch in &chapters {
        assert!(
            paper.contains(&ch.title),
            "ToC should reference chapter '{}'",
            ch.title,
        );
    }
}

#[test]
fn test_combiner_chapter_order() {
    let chapters = sample_chapters();
    let paper = combine_chapters(&chapters);

    // Verify titles present (ToC or chapter headings)
    assert!(paper.contains("Introduction"));
    assert!(paper.contains("Literature Review"));
    assert!(paper.contains("Methodology"));

    // After the ToC, the actual chapter content should appear in order.
    // Find the positions of the chapter *content* (not ToC entries).
    let content_pos_intro = paper
        .find("This chapter introduces the research topic")
        .expect("should contain intro content");
    let content_pos_lit = paper
        .find("This chapter reviews existing literature")
        .expect("should contain lit content");
    let content_pos_meth = paper
        .find("This chapter describes the methodology")
        .expect("should contain meth content");

    assert!(
        content_pos_intro < content_pos_lit,
        "Introduction content should appear before Literature Review content"
    );
    assert!(
        content_pos_lit < content_pos_meth,
        "Literature Review content should appear before Methodology content"
    );
}

// ---------------------------------------------------------------------------
// Options tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_dry_run_mode() {
    let files = standard_agent_files();
    let dir = create_agent_output_dir(&files);

    let mut options = default_options();
    options.dry_run = true;

    let orchestrator = FinalStageOrchestrator::new();
    let result = orchestrator
        .run(dir.path(), &sample_chapter_list(), &options)
        .await;

    match result {
        Ok(res) => {
            // In dry_run mode, no final paper should be written.
            assert!(
                res.final_paper_path.is_empty()
                    || !std::path::Path::new(&res.final_paper_path).exists(),
                "dry_run should not produce a final paper on disk"
            );
            // But we should still get a chapter count from the mapping phase.
            assert!(
                res.chapter_count > 0,
                "dry_run should still report chapter_count from mapping"
            );
        }
        Err(e) => panic!("dry_run should succeed, got error: {:?}", e),
    }
}

#[tokio::test]
async fn test_force_overwrites() {
    let files = standard_agent_files();
    let dir = create_agent_output_dir(&files);

    // Create a pre-existing "final" directory to simulate prior output.
    let final_dir = dir.path().join("final");
    fs::create_dir_all(&final_dir).unwrap();
    fs::write(final_dir.join("final-paper.md"), "old paper content").unwrap();

    let mut options = default_options();
    options.force = true;

    let orchestrator = FinalStageOrchestrator::new();
    let result = orchestrator
        .run(dir.path(), &sample_chapter_list(), &options)
        .await;

    // With force=true, the existing output should be overwritten, not produce OutputExists error.
    assert!(
        !matches!(result, Err(FinalStageError::OutputExists)),
        "force=true should not produce OutputExists error; got: {:?}",
        result,
    );
}

// ---------------------------------------------------------------------------
// Recovery strategy tests
// ---------------------------------------------------------------------------

#[test]
fn test_recovery_strategy_scan_retry() {
    // ScanFailed should allow up to 2 retries.
    let error = FinalStageError::ScanFailed("permission denied".to_string());
    let max_retries = FinalStageOrchestrator::max_retries_for(&error);
    assert_eq!(
        max_retries, 2,
        "ScanFailed should allow 2 retries, got {}",
        max_retries,
    );
}

#[test]
fn test_recovery_strategy_token_abort() {
    // TokenOverflow is non-recoverable — zero retries.
    let error = FinalStageError::TokenOverflow;
    let max_retries = FinalStageOrchestrator::max_retries_for(&error);
    assert_eq!(
        max_retries, 0,
        "TokenOverflow should be non-recoverable (0 retries), got {}",
        max_retries,
    );
}

#[test]
fn test_recovery_strategy_write_error_retry() {
    let error = FinalStageError::WriteError("disk full".to_string());
    let max_retries = FinalStageOrchestrator::max_retries_for(&error);
    assert_eq!(
        max_retries, 1,
        "WriteError should allow 1 retry, got {}",
        max_retries,
    );
}

#[test]
fn test_recovery_strategy_style_error_skip() {
    let error = FinalStageError::StyleError("unknown profile".to_string());
    let max_retries = FinalStageOrchestrator::max_retries_for(&error);
    assert_eq!(
        max_retries, 0,
        "StyleError should be skippable (0 retries), got {}",
        max_retries,
    );
}

// ---------------------------------------------------------------------------
// Additional edge-case tests
// ---------------------------------------------------------------------------

#[test]
fn test_invalid_reverse_transitions() {
    // Going backward in the state machine should be invalid.
    let invalid_pairs = [
        (FinalStageState::Scanning, FinalStageState::Idle),
        (FinalStageState::Writing, FinalStageState::Scanning),
        (FinalStageState::Completed, FinalStageState::Writing),
        (FinalStageState::Combining, FinalStageState::Initializing),
    ];

    for (from, to) in &invalid_pairs {
        let result = FinalStageOrchestrator::validate_transition(*from, *to);
        assert!(
            result.is_err(),
            "reverse transition {:?} -> {:?} should be invalid",
            from,
            to,
        );
    }
}

#[test]
fn test_scanner_parses_filename_index_correctly() {
    let dir = create_agent_output_dir(&[
        ("10-late-phase-agent.md", "# Late phase content"),
        ("07-mid-phase-agent.md", "# Mid phase content"),
    ]);

    let outputs = scan_outputs(dir.path()).expect("should scan outputs");
    assert_eq!(outputs.len(), 2);

    let late = outputs
        .iter()
        .find(|o| o.agent_key == "late-phase-agent")
        .expect("should find late-phase-agent");
    assert_eq!(late.phase, 10, "index 10 should parse as phase 10");

    let mid = outputs
        .iter()
        .find(|o| o.agent_key == "mid-phase-agent")
        .expect("should find mid-phase-agent");
    assert_eq!(mid.phase, 7, "index 07 should parse as phase 7");
}
