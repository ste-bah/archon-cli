//! Integration tests for ChapterStructureLoader and DynamicAgentGenerator.
//!
//! Tests TASK-PIPE-C05: research::chapters module.

use archon_pipeline::research::chapters::{
    ChapterDefinition, ChapterStructureError, ChapterStructureLoader, DynamicAgentGenerator,
};

// ---------------------------------------------------------------------------
// Test data helpers
// ---------------------------------------------------------------------------

/// Returns a realistic 6-chapter JSON wrapped in a ```json code block.
fn six_chapter_json_codeblock() -> String {
    format!("```json\n{}\n```", six_chapter_raw_json())
}

/// Returns a realistic 6-chapter raw JSON string (no code block wrapper).
fn six_chapter_raw_json() -> String {
    r#"{
  "locked": true,
  "generatedAt": "2026-04-07T10:00:00Z",
  "totalChapters": 6,
  "estimatedTotalWords": 30000,
  "chapters": [
    {"number": 1, "title": "Introduction", "writerAgent": "introduction-writer", "targetWords": 5000, "sections": ["Background", "Problem Statement", "Research Questions"], "outputFile": "chapter-01.md"},
    {"number": 2, "title": "Literature Review", "writerAgent": "literature-review-writer", "targetWords": 8000, "sections": ["Theoretical Framework", "Prior Work", "Research Gap"], "outputFile": "chapter-02.md"},
    {"number": 3, "title": "Methodology", "writerAgent": "methodology-writer", "targetWords": 5000, "sections": ["Research Design", "Data Collection", "Analysis Approach"], "outputFile": "chapter-03.md"},
    {"number": 4, "title": "Results", "writerAgent": "results-writer", "targetWords": 6000, "sections": ["Quantitative Findings", "Qualitative Findings"], "outputFile": "chapter-04.md"},
    {"number": 5, "title": "Discussion", "writerAgent": "discussion-writer", "targetWords": 4000, "sections": ["Interpretation", "Implications", "Limitations"], "outputFile": "chapter-05.md"},
    {"number": 6, "title": "Conclusion", "writerAgent": "conclusion-writer", "targetWords": 2000, "sections": ["Summary", "Future Work"], "outputFile": "chapter-06.md"}
  ],
  "writerMapping": {
    "introduction-writer": "Introduction",
    "literature-review-writer": "Literature Review",
    "methodology-writer": "Methodology",
    "results-writer": "Results",
    "discussion-writer": "Discussion",
    "conclusion-writer": "Conclusion"
  }
}"#
    .to_string()
}

/// Returns a 10-chapter raw JSON string.
fn ten_chapter_raw_json() -> String {
    r#"{
  "locked": true,
  "generatedAt": "2026-04-07T12:00:00Z",
  "totalChapters": 10,
  "estimatedTotalWords": 60000,
  "chapters": [
    {"number": 1, "title": "Abstract", "writerAgent": "abstract-writer", "targetWords": 1000, "sections": ["Overview"], "outputFile": "chapter-01.md"},
    {"number": 2, "title": "Introduction", "writerAgent": "introduction-writer", "targetWords": 5000, "sections": ["Background", "Objectives"], "outputFile": "chapter-02.md"},
    {"number": 3, "title": "Literature Review", "writerAgent": "literature-review-writer", "targetWords": 8000, "sections": ["Prior Art"], "outputFile": "chapter-03.md"},
    {"number": 4, "title": "Theoretical Framework", "writerAgent": "chapter-synthesizer", "targetWords": 6000, "sections": ["Models", "Hypotheses"], "outputFile": "chapter-04.md"},
    {"number": 5, "title": "Methodology", "writerAgent": "methodology-writer", "targetWords": 5000, "sections": ["Design", "Instruments"], "outputFile": "chapter-05.md"},
    {"number": 6, "title": "Results", "writerAgent": "results-writer", "targetWords": 7000, "sections": ["Quantitative", "Qualitative"], "outputFile": "chapter-06.md"},
    {"number": 7, "title": "Discussion", "writerAgent": "discussion-writer", "targetWords": 6000, "sections": ["Analysis"], "outputFile": "chapter-07.md"},
    {"number": 8, "title": "Implications", "writerAgent": "chapter-synthesizer", "targetWords": 4000, "sections": ["Practical", "Theoretical"], "outputFile": "chapter-08.md"},
    {"number": 9, "title": "Conclusion", "writerAgent": "conclusion-writer", "targetWords": 3000, "sections": ["Summary", "Outlook"], "outputFile": "chapter-09.md"},
    {"number": 10, "title": "Appendix", "writerAgent": "chapter-synthesizer", "targetWords": 2000, "sections": ["Data Tables"], "outputFile": "chapter-10.md"}
  ],
  "writerMapping": {
    "abstract-writer": "Abstract",
    "introduction-writer": "Introduction",
    "literature-review-writer": "Literature Review",
    "chapter-synthesizer": "Theoretical Framework",
    "methodology-writer": "Methodology",
    "results-writer": "Results",
    "discussion-writer": "Discussion",
    "conclusion-writer": "Conclusion"
  }
}"#
    .to_string()
}

/// Returns an unlocked structure JSON (locked: false).
fn unlocked_json() -> String {
    r#"{
  "locked": false,
  "generatedAt": "2026-04-07T10:00:00Z",
  "totalChapters": 1,
  "estimatedTotalWords": 5000,
  "chapters": [
    {"number": 1, "title": "Introduction", "writerAgent": "introduction-writer", "targetWords": 5000, "sections": ["Background"], "outputFile": "chapter-01.md"}
  ],
  "writerMapping": {}
}"#
    .to_string()
}

/// Returns a JSON missing the "locked" field entirely.
fn missing_locked_json() -> String {
    r#"{
  "generatedAt": "2026-04-07T10:00:00Z",
  "totalChapters": 1,
  "estimatedTotalWords": 5000,
  "chapters": [
    {"number": 1, "title": "Introduction", "writerAgent": "introduction-writer", "targetWords": 5000, "sections": ["Background"], "outputFile": "chapter-01.md"}
  ],
  "writerMapping": {}
}"#
    .to_string()
}

/// Returns a chapter entry missing the required "title" field.
fn missing_title_json() -> String {
    r#"{
  "locked": true,
  "generatedAt": "2026-04-07T10:00:00Z",
  "totalChapters": 1,
  "estimatedTotalWords": 5000,
  "chapters": [
    {"number": 1, "writerAgent": "introduction-writer", "targetWords": 5000, "sections": ["Background"], "outputFile": "chapter-01.md"}
  ],
  "writerMapping": {}
}"#
    .to_string()
}

/// Returns JSON using legacy field names (assignedAgent, wordTarget, dateLocked).
fn legacy_fields_json() -> String {
    r#"{
  "locked": true,
  "dateLocked": "2026-04-07T10:00:00Z",
  "totalChapters": 2,
  "estimatedTotalWords": 10000,
  "chapters": [
    {"number": 1, "title": "Introduction", "assignedAgent": "introduction-writer", "wordTarget": 5000, "sections": ["Background"], "outputFile": "chapter-01.md"},
    {"number": 2, "title": "Conclusion", "assignedAgent": "conclusion-writer", "wordTarget": 5000, "sections": ["Summary"], "outputFile": "chapter-02.md"}
  ],
  "writerMapping": {}
}"#
    .to_string()
}

/// Returns markdown fallback content (no JSON).
fn markdown_chapters() -> String {
    r#"### Chapter 1: Introduction
**Purpose:** Introduce the research topic
**Content Outline:** Background, Problem Statement
**Word Count Target:** 5000

### Chapter 2: Literature Review
**Purpose:** Review existing research
**Content Outline:** Theoretical Framework, Prior Work
**Word Count Target:** 8000"#
        .to_string()
}

// ---------------------------------------------------------------------------
// ChapterStructureLoader — parsing tests
// ---------------------------------------------------------------------------

#[test]
fn test_parse_locked_6_chapter_json_codeblock() {
    let content = six_chapter_json_codeblock();
    let structure = ChapterStructureLoader::parse_structure(&content)
        .expect("should parse 6-chapter JSON code block");

    assert!(structure.locked);
    assert_eq!(structure.total_chapters, 6);
    assert_eq!(structure.chapters.len(), 6);
    assert_eq!(structure.estimated_total_words, 30000);
    assert_eq!(structure.chapters[0].title, "Introduction");
    assert_eq!(structure.chapters[0].number, 1);
    assert_eq!(structure.chapters[0].target_words, 5000);
    assert_eq!(structure.chapters[0].sections.len(), 3);
    assert_eq!(structure.chapters[0].output_file, "chapter-01.md");
    assert_eq!(structure.chapters[5].title, "Conclusion");
    assert_eq!(structure.chapters[5].number, 6);
    assert_eq!(structure.chapters[5].target_words, 2000);
}

#[test]
fn test_parse_locked_10_chapter_structure() {
    let content = ten_chapter_raw_json();
    let structure = ChapterStructureLoader::parse_structure(&content)
        .expect("should parse 10-chapter structure");

    assert!(structure.locked);
    assert_eq!(structure.total_chapters, 10);
    assert_eq!(structure.chapters.len(), 10);
    assert_eq!(structure.estimated_total_words, 60000);

    // Spot-check first and last chapters.
    assert_eq!(structure.chapters[0].title, "Abstract");
    assert_eq!(structure.chapters[9].title, "Appendix");
    assert_eq!(structure.chapters[9].number, 10);
}

// ---------------------------------------------------------------------------
// ChapterStructureLoader — rejection / error tests
// ---------------------------------------------------------------------------

#[test]
fn test_reject_unlocked_structure() {
    let content = unlocked_json();
    let err = ChapterStructureLoader::parse_structure(&content)
        .expect_err("should reject unlocked structure");

    match err {
        ChapterStructureError::NotLocked => {} // expected
        other => panic!("expected NotLocked, got: {:?}", other),
    }
}

#[test]
fn test_reject_missing_locked_field() {
    let content = missing_locked_json();
    let err = ChapterStructureLoader::parse_structure(&content)
        .expect_err("should reject structure missing locked field");

    match err {
        ChapterStructureError::NotLocked => {} // expected
        other => panic!("expected NotLocked, got: {:?}", other),
    }
}

#[test]
fn test_reject_malformed_json() {
    let content = "```json\n{ this is not valid json }\n```";
    let err =
        ChapterStructureLoader::parse_structure(content).expect_err("should reject malformed JSON");

    match err {
        ChapterStructureError::ParseError(_) => {} // expected
        other => panic!("expected ParseError, got: {:?}", other),
    }
}

#[test]
fn test_reject_missing_required_field() {
    let content = missing_title_json();
    let err = ChapterStructureLoader::parse_structure(&content)
        .expect_err("should reject chapter missing title");

    match err {
        ChapterStructureError::InvalidDefinition { index, field } => {
            assert_eq!(index, 0, "should reference first chapter");
            assert_eq!(field, "title", "should identify missing field");
        }
        other => panic!("expected InvalidDefinition, got: {:?}", other),
    }
}

// ---------------------------------------------------------------------------
// ChapterStructureLoader — legacy field normalization
// ---------------------------------------------------------------------------

#[test]
fn test_legacy_field_normalization() {
    let content = legacy_fields_json();
    let raw: serde_json::Value =
        serde_json::from_str(&content).expect("test data should be valid JSON");
    let structure =
        ChapterStructureLoader::normalize_structure(raw).expect("should normalize legacy fields");

    assert!(structure.locked);
    // dateLocked -> generated_at
    assert_eq!(structure.generated_at, "2026-04-07T10:00:00Z");
    // assignedAgent -> writer_agent
    assert_eq!(structure.chapters[0].writer_agent, "introduction-writer");
    assert_eq!(structure.chapters[1].writer_agent, "conclusion-writer");
    // wordTarget -> target_words
    assert_eq!(structure.chapters[0].target_words, 5000);
    assert_eq!(structure.chapters[1].target_words, 5000);
}

// ---------------------------------------------------------------------------
// ChapterStructureLoader — markdown fallback
// ---------------------------------------------------------------------------

#[test]
fn test_markdown_fallback_parsing() {
    let content = markdown_chapters();
    let structure = ChapterStructureLoader::parse_from_markdown(&content)
        .expect("should parse markdown chapter headings");

    assert_eq!(structure.chapters.len(), 2);
    assert_eq!(structure.chapters[0].number, 1);
    assert_eq!(structure.chapters[0].title, "Introduction");
    assert_eq!(structure.chapters[0].target_words, 5000);
    assert!(
        structure.chapters[0]
            .sections
            .iter()
            .any(|s: &String| s.contains("Background")),
        "sections should include Background"
    );

    assert_eq!(structure.chapters[1].number, 2);
    assert_eq!(structure.chapters[1].title, "Literature Review");
    assert_eq!(structure.chapters[1].target_words, 8000);
}

// ---------------------------------------------------------------------------
// ChapterStructureLoader — writer inference
// ---------------------------------------------------------------------------

#[test]
fn test_infer_writer_introduction() {
    assert_eq!(
        ChapterStructureLoader::infer_writer_agent(1, "Introduction"),
        "introduction-writer"
    );
}

#[test]
fn test_infer_writer_literature() {
    assert_eq!(
        ChapterStructureLoader::infer_writer_agent(2, "Literature Review"),
        "literature-review-writer"
    );
}

#[test]
fn test_infer_writer_methodology() {
    assert_eq!(
        ChapterStructureLoader::infer_writer_agent(3, "Methodology"),
        "methodology-writer"
    );
    // Also test the "Methods" variant.
    assert_eq!(
        ChapterStructureLoader::infer_writer_agent(3, "Methods"),
        "methodology-writer"
    );
}

#[test]
fn test_infer_writer_results() {
    assert_eq!(
        ChapterStructureLoader::infer_writer_agent(4, "Results"),
        "results-writer"
    );
    // Also test the "Findings" variant.
    assert_eq!(
        ChapterStructureLoader::infer_writer_agent(4, "Key Findings"),
        "results-writer"
    );
}

#[test]
fn test_infer_writer_discussion() {
    assert_eq!(
        ChapterStructureLoader::infer_writer_agent(5, "Discussion"),
        "discussion-writer"
    );
}

#[test]
fn test_infer_writer_conclusion() {
    assert_eq!(
        ChapterStructureLoader::infer_writer_agent(6, "Conclusion"),
        "conclusion-writer"
    );
}

#[test]
fn test_infer_writer_unknown() {
    assert_eq!(
        ChapterStructureLoader::infer_writer_agent(10, "Appendix"),
        "chapter-synthesizer"
    );
}

// ---------------------------------------------------------------------------
// ChapterStructureLoader — validation
// ---------------------------------------------------------------------------

#[test]
fn test_validate_chapter_rejects_empty_title() {
    let chapter = ChapterDefinition {
        number: 1,
        title: String::new(),
        writer_agent: "introduction-writer".to_string(),
        target_words: 5000,
        sections: vec!["Background".to_string()],
        output_file: "chapter-01.md".to_string(),
    };

    let err = ChapterStructureLoader::validate_chapter(&chapter, 0)
        .expect_err("should reject empty title");

    match err {
        ChapterStructureError::InvalidDefinition { index, field } => {
            assert_eq!(index, 0);
            assert_eq!(field, "title");
        }
        other => panic!("expected InvalidDefinition for title, got: {:?}", other),
    }
}

// ---------------------------------------------------------------------------
// DynamicAgentGenerator — agent count
// ---------------------------------------------------------------------------

#[test]
fn test_dynamic_agent_count_matches_chapters() {
    let content = six_chapter_raw_json();
    let structure = ChapterStructureLoader::parse_structure(&content)
        .expect("should parse 6-chapter structure");

    let agents = DynamicAgentGenerator::generate_writing_agents(&structure);
    assert_eq!(
        agents.len(),
        structure.chapters.len(),
        "should generate exactly one agent per chapter"
    );
}

// ---------------------------------------------------------------------------
// DynamicAgentGenerator — tool access
// ---------------------------------------------------------------------------

#[test]
fn test_generated_agents_have_write_access() {
    let content = six_chapter_raw_json();
    let structure =
        ChapterStructureLoader::parse_structure(&content).expect("should parse structure");

    let agents = DynamicAgentGenerator::generate_writing_agents(&structure);

    let required_tools = ["Write", "Read", "Glob", "Grep"];
    for agent in &agents {
        for tool in &required_tools {
            assert!(
                agent.tool_access.contains(&tool.to_string()),
                "agent {} missing required tool: {}",
                agent.agent_key,
                tool
            );
        }
    }
}

// ---------------------------------------------------------------------------
// DynamicAgentGenerator — chapter context on agents
// ---------------------------------------------------------------------------

#[test]
fn test_generated_agent_has_chapter_context() {
    let content = six_chapter_raw_json();
    let structure =
        ChapterStructureLoader::parse_structure(&content).expect("should parse structure");

    let agents = DynamicAgentGenerator::generate_writing_agents(&structure);

    // Check the first agent maps to chapter 1.
    let agent = &agents[0];
    assert_eq!(agent.chapter_number, 1);
    assert_eq!(agent.chapter_title, "Introduction");
    assert_eq!(agent.target_words, 5000);
    assert_eq!(
        agent.sections,
        vec!["Background", "Problem Statement", "Research Questions"]
    );

    // Check the last agent maps to chapter 6.
    let last = &agents[5];
    assert_eq!(last.chapter_number, 6);
    assert_eq!(last.chapter_title, "Conclusion");
    assert_eq!(last.target_words, 2000);
    assert_eq!(last.sections, vec!["Summary", "Future Work"]);
}

// ---------------------------------------------------------------------------
// DynamicAgentGenerator — output path pattern
// ---------------------------------------------------------------------------

#[test]
fn test_generated_agent_output_path() {
    let content = six_chapter_raw_json();
    let structure =
        ChapterStructureLoader::parse_structure(&content).expect("should parse structure");

    let agents = DynamicAgentGenerator::generate_writing_agents(&structure);

    // Each agent's output_path should end with the chapter's output_file.
    for (i, agent) in agents.iter().enumerate() {
        let expected_filename = format!("chapter-{:02}.md", i + 1);
        assert!(
            agent.output_path.ends_with(&expected_filename),
            "agent {} output_path '{}' should end with '{}'",
            agent.agent_key,
            agent.output_path,
            expected_filename
        );
    }
}
