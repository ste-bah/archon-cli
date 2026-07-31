//! Tests for the 47-agent research pipeline definitions.
use archon_pipeline::research::agents::*;
use std::collections::HashSet;
#[test]
fn test_agent_count() {
    assert_eq!(
        RESEARCH_AGENTS.len(),
        47,
        "Research pipeline must have exactly 47 agents"
    );
}
#[test]
fn test_no_duplicate_keys() {
    let mut seen = HashSet::new();
    for agent in RESEARCH_AGENTS.iter() {
        assert!(seen.insert(agent.key), "Duplicate agent key: {}", agent.key);
    }
}
#[test]
fn test_all_agents_have_web_tools() {
    let required = [
        ResearchToolAccess::WebSearch,
        ResearchToolAccess::WebFetch,
        ResearchToolAccess::Read,
        ResearchToolAccess::Glob,
        ResearchToolAccess::Grep,
    ];
    for agent in RESEARCH_AGENTS.iter() {
        for tool in &required {
            assert!(
                agent.tool_access.contains(tool),
                "Agent '{}' missing required tool {:?}",
                agent.key,
                tool
            );
        }
    }
}
#[test]
fn test_writing_agents_have_write() {
    let write_agent_keys: HashSet<&str> = [
        "introduction-writer",
        "literature-review-writer",
        "results-writer",
        "discussion-writer",
        "conclusion-writer",
        "abstract-writer",
        "chapter-synthesizer",
    ]
    .into_iter()
    .collect();
    for agent in RESEARCH_AGENTS.iter() {
        if agent.phase == 6 || agent.phase == 8 {
            assert!(
                write_agent_keys.contains(agent.key),
                "Unexpected writing/final assembly agent: {}",
                agent.key
            );
            assert!(
                agent.tool_access.contains(&ResearchToolAccess::Write),
                "Writing/final assembly agent '{}' must have Write tool access",
                agent.key
            );
        }
    }
    // Also verify non-writing agents do NOT have Write.
    for agent in RESEARCH_AGENTS.iter() {
        if !write_agent_keys.contains(agent.key) {
            assert!(
                !agent.tool_access.contains(&ResearchToolAccess::Write),
                "Non-writing agent '{}' should not have Write tool access",
                agent.key
            );
        }
    }
}
#[test]
fn test_phase_counts() {
    let mut counts = [0u32; 9]; // index 0 unused, 1-8 for phases
    for agent in RESEARCH_AGENTS.iter() {
        assert!(
            (1..=8).contains(&agent.phase),
            "Agent '{}' has invalid phase {}",
            agent.key,
            agent.phase
        );
        counts[agent.phase as usize] += 1;
    }
    assert_eq!(counts[1], 6, "Phase 1 should have 6 agents");
    assert_eq!(counts[2], 4, "Phase 2 should have 4 agents");
    assert_eq!(counts[3], 4, "Phase 3 should have 4 agents");
    assert_eq!(counts[4], 5, "Phase 4 should have 5 agents");
    assert_eq!(counts[5], 9, "Phase 5 should have 9 agents");
    assert_eq!(counts[6], 6, "Phase 6 should have 6 agents");
    assert_eq!(counts[7], 12, "Phase 7 should have 12 agents");
    assert_eq!(counts[8], 1, "Phase 8 should have 1 agent");
}
#[test]
fn test_validate_configuration() {
    assert!(
        validate_configuration().is_ok(),
        "Configuration validation must pass: {:?}",
        validate_configuration()
    );
}
#[test]
fn test_agent_lookup_by_key() {
    let agent = get_agent_by_key("step-back-analyzer");
    assert!(agent.is_some(), "step-back-analyzer must be found");
    let agent = agent.unwrap();
    assert_eq!(agent.key, "step-back-analyzer");
    assert_eq!(agent.display_name, "Step-Back Analyzer");
    assert_eq!(agent.phase, 1);
    // Check a phase 7 agent
    let agent = get_agent_by_key("file-length-manager");
    assert!(agent.is_some(), "file-length-manager must be found");
    let agent = agent.unwrap();
    assert_eq!(agent.phase, 7);
    // Check final assembly
    let agent = get_agent_by_key("chapter-synthesizer");
    assert!(agent.is_some(), "chapter-synthesizer must be found");
    let agent = agent.unwrap();
    assert_eq!(agent.phase, 8);
    // Non-existent key
    assert!(get_agent_by_key("nonexistent").is_none());
}
#[test]
fn test_get_agents_by_phase() {
    assert_eq!(get_agents_by_phase(1).len(), 6);
    assert_eq!(get_agents_by_phase(2).len(), 4);
    assert_eq!(get_agents_by_phase(3).len(), 4);
    assert_eq!(get_agents_by_phase(4).len(), 5);
    assert_eq!(get_agents_by_phase(5).len(), 9);
    assert_eq!(get_agents_by_phase(6).len(), 6);
    assert_eq!(get_agents_by_phase(7).len(), 12);
    assert_eq!(get_agents_by_phase(8).len(), 1);
}
#[test]
fn test_get_agent_index() {
    assert_eq!(get_agent_index("step-back-analyzer"), Some(0));
    assert_eq!(get_agent_index("self-ask-decomposer"), Some(1));
    assert_eq!(get_agent_index("citation-reconciler"), Some(41));
    assert_eq!(get_agent_index("file-length-manager"), Some(45));
    assert_eq!(get_agent_index("chapter-synthesizer"), Some(46));
    assert_eq!(get_agent_index("nonexistent"), None);
}
#[test]
fn test_get_phase_by_id() {
    let phase = get_phase_by_id(1);
    assert!(phase.is_some());
    let phase = phase.unwrap();
    assert_eq!(phase.id, 1);
    assert_eq!(phase.name, "Foundation");
    assert_eq!(phase.agent_keys.len(), 6);
    let phase = get_phase_by_id(7).unwrap();
    assert_eq!(phase.name, "Validation");
    assert_eq!(phase.agent_keys.len(), 12);
    let phase = get_phase_by_id(8).unwrap();
    assert_eq!(phase.name, "Final Assembly");
    assert_eq!(phase.agent_keys, &["chapter-synthesizer"]);
    assert!(get_phase_by_id(0).is_none());
    assert!(get_phase_by_id(9).is_none());
}
#[test]
fn test_memory_keys_non_empty() {
    for agent in RESEARCH_AGENTS.iter() {
        assert!(
            !agent.memory_keys.is_empty(),
            "Agent '{}' must have at least 1 memory key",
            agent.key
        );
    }
}
#[test]
fn test_output_artifacts_non_empty() {
    for agent in RESEARCH_AGENTS.iter() {
        assert!(
            !agent.output_artifacts.is_empty(),
            "Agent '{}' must have at least 1 output artifact",
            agent.key
        );
    }
}
#[test]
fn test_citation_reconciler_owns_final_reference_context() {
    let reconciler = get_agent_by_key("citation-reconciler").unwrap();
    assert!(
        reconciler
            .memory_keys
            .contains(&"research/quality/citations")
    );
    assert!(
        reconciler
            .memory_keys
            .contains(&"research/document/references")
    );
    assert!(
        reconciler
            .memory_keys
            .contains(&"research/quality/citation-repair")
    );
    let consistency = get_agent_by_key("consistency-validator").unwrap();
    assert!(
        !consistency
            .memory_keys
            .contains(&"research/quality/citations"),
        "consistency validator must not overwrite repaired citation context"
    );
    let finalizer = get_agent_by_key("chapter-synthesizer").unwrap();
    assert!(
        finalizer
            .memory_keys
            .contains(&"research/quality/citation-repair")
    );
    assert!(
        finalizer
            .memory_keys
            .contains(&"research/document/references")
    );
}
#[test]
fn test_prompt_source_paths() {
    for agent in RESEARCH_AGENTS.iter() {
        let expected = format!(".archon/agents/phdresearch/{}.md", agent.key);
        assert_eq!(
            agent.prompt_source_path, expected,
            "Agent '{}' has wrong prompt_source_path: expected '{}', got '{}'",
            agent.key, expected, agent.prompt_source_path
        );
    }
}
#[test]
fn test_phase_names() {
    let expected = [
        (1, "Foundation"),
        (2, "Discovery"),
        (3, "Architecture"),
        (4, "Synthesis"),
        (5, "Design"),
        (6, "Writing"),
        (7, "Validation"),
        (8, "Final Assembly"),
    ];
    for (id, name) in &expected {
        let phase = get_phase_by_id(*id).unwrap();
        assert_eq!(phase.name, *name);
    }
}
#[test]
fn test_research_phases_count() {
    assert_eq!(RESEARCH_PHASES.len(), 8, "Must have exactly 8 phases");
}
#[test]
fn test_serialization_roundtrip() {
    // Verify agents can be serialized to JSON
    let json = serde_json::to_string(&RESEARCH_AGENTS[0]).unwrap();
    assert!(json.contains("step-back-analyzer"));
    // Verify phases can be serialized
    let json = serde_json::to_string(&RESEARCH_PHASES[0]).unwrap();
    assert!(json.contains("Foundation"));
}
const EXPECTED_RESEARCH_KEYS: &[&str] = &[
    r###"step-back-analyzer"###,
    r###"self-ask-decomposer"###,
    r###"ambiguity-clarifier"###,
    r###"research-planner"###,
    r###"construct-definer"###,
    r###"dissertation-architect"###,
    r###"literature-mapper"###,
    r###"source-tier-classifier"###,
    r###"citation-extractor"###,
    r###"context-tier-manager"###,
    r###"theoretical-framework-analyst"###,
    r###"contradiction-analyzer"###,
    r###"gap-hunter"###,
    r###"risk-analyst"###,
    r###"evidence-synthesizer"###,
    r###"pattern-analyst"###,
    r###"thematic-synthesizer"###,
    r###"theory-builder"###,
    r###"opportunity-identifier"###,
    r###"method-designer"###,
    r###"hypothesis-generator"###,
    r###"model-architect"###,
    r###"analysis-planner"###,
    r###"sampling-strategist"###,
    r###"instrument-developer"###,
    r###"validity-guardian"###,
    r###"methodology-scanner"###,
    r###"methodology-writer"###,
    r###"introduction-writer"###,
    r###"literature-review-writer"###,
    r###"results-writer"###,
    r###"discussion-writer"###,
    r###"conclusion-writer"###,
    r###"abstract-writer"###,
    r###"systematic-reviewer"###,
    r###"ethics-reviewer"###,
    r###"adversarial-reviewer"###,
    r###"confidence-quantifier"###,
    r###"citation-validator"###,
    r###"reproducibility-checker"###,
    r###"apa-citation-specialist"###,
    r###"citation-reconciler"###,
    r###"consistency-validator"###,
    r###"quality-assessor"###,
    r###"bias-detector"###,
    r###"file-length-manager"###,
    r###"chapter-synthesizer"###,
];
const EXPECTED_RESEARCH_JSON: &[&str] = &[
    r###"{"key":"step-back-analyzer","display_name":"Step-Back Analyzer","phase":1,"file":"step-back-analyzer.md","memory_keys":["research/foundation/framing","research/meta/perspective"],"output_artifacts":["high-level-framing.md","abstraction-analysis.md"],"prompt_source_path":".archon/agents/phdresearch/step-back-analyzer.md","tool_access":["WebSearch","WebFetch","Read","Glob","Grep"]}"###,
    r###"{"key":"self-ask-decomposer","display_name":"Self-Ask Decomposer","phase":1,"file":"self-ask-decomposer.md","memory_keys":["research/meta/questions","research/foundation/decomposition"],"output_artifacts":["essential-questions.md","knowledge-gaps.md"],"prompt_source_path":".archon/agents/phdresearch/self-ask-decomposer.md","tool_access":["WebSearch","WebFetch","Read","Glob","Grep"]}"###,
    r###"{"key":"ambiguity-clarifier","display_name":"Ambiguity Clarifier","phase":1,"file":"ambiguity-clarifier.md","memory_keys":["research/foundation/definitions","research/meta/clarifications"],"output_artifacts":["term-definitions.md","clarified-scope.md"],"prompt_source_path":".archon/agents/phdresearch/ambiguity-clarifier.md","tool_access":["WebSearch","WebFetch","Read","Glob","Grep"]}"###,
    r###"{"key":"research-planner","display_name":"Research Planner","phase":1,"file":"research-planner.md","memory_keys":["research/foundation/plan","research/meta/strategy"],"output_artifacts":["research-plan.md","timeline.md"],"prompt_source_path":".archon/agents/phdresearch/research-planner.md","tool_access":["WebSearch","WebFetch","Read","Glob","Grep"]}"###,
    r###"{"key":"construct-definer","display_name":"Construct Definer","phase":1,"file":"construct-definer.md","memory_keys":["research/foundation/constructs","research/theory/definitions"],"output_artifacts":["construct-definitions.md","operationalizations.md"],"prompt_source_path":".archon/agents/phdresearch/construct-definer.md","tool_access":["WebSearch","WebFetch","Read","Glob","Grep"]}"###,
    r###"{"key":"dissertation-architect","display_name":"Dissertation Architect","phase":1,"file":"dissertation-architect.md","memory_keys":["research/structure/chapters","research/writing/structure","research/document/architecture"],"output_artifacts":["dissertation-outline.md","chapter-structure.md"],"prompt_source_path":".archon/agents/phdresearch/dissertation-architect.md","tool_access":["WebSearch","WebFetch","Read","Glob","Grep"]}"###,
    r###"{"key":"literature-mapper","display_name":"Literature Mapper","phase":2,"file":"literature-mapper.md","memory_keys":["research/literature/map","research/sources/index"],"output_artifacts":["literature-map.md","source-catalog.md"],"prompt_source_path":".archon/agents/phdresearch/literature-mapper.md","tool_access":["WebSearch","WebFetch","Read","Glob","Grep"]}"###,
    r###"{"key":"source-tier-classifier","display_name":"Source Tier Classifier","phase":2,"file":"source-tier-classifier.md","memory_keys":["research/literature/tiers","research/quality/sources"],"output_artifacts":["source-tiers.md","credibility-assessment.md"],"prompt_source_path":".archon/agents/phdresearch/source-tier-classifier.md","tool_access":["WebSearch","WebFetch","Read","Glob","Grep"]}"###,
    r###"{"key":"citation-extractor","display_name":"Citation Extractor","phase":2,"file":"citation-extractor.md","memory_keys":["research/quality/extraction","research/sources/citations"],"output_artifacts":["extracted-citations.md","reference-list.md"],"prompt_source_path":".archon/agents/phdresearch/citation-extractor.md","tool_access":["WebSearch","WebFetch","Read","Glob","Grep"]}"###,
    r###"{"key":"context-tier-manager","display_name":"Context Tier Manager","phase":2,"file":"context-tier-manager.md","memory_keys":["research/literature/context","research/meta/tiers"],"output_artifacts":["context-hierarchy.md","tier-mappings.md"],"prompt_source_path":".archon/agents/phdresearch/context-tier-manager.md","tool_access":["WebSearch","WebFetch","Read","Glob","Grep"]}"###,
    r###"{"key":"theoretical-framework-analyst","display_name":"Theoretical Framework Analyst","phase":3,"file":"theoretical-framework-analyst.md","memory_keys":["research/foundation/framework","research/theory/analysis"],"output_artifacts":["theoretical-framework.md","framework-map.md"],"prompt_source_path":".archon/agents/phdresearch/theoretical-framework-analyst.md","tool_access":["WebSearch","WebFetch","Read","Glob","Grep"]}"###,
    r###"{"key":"contradiction-analyzer","display_name":"Contradiction Analyzer","phase":3,"file":"contradiction-analyzer.md","memory_keys":["research/analysis/contradictions","research/findings/conflicts"],"output_artifacts":["contradictions-report.md","resolution-proposals.md"],"prompt_source_path":".archon/agents/phdresearch/contradiction-analyzer.md","tool_access":["WebSearch","WebFetch","Read","Glob","Grep"]}"###,
    r###"{"key":"gap-hunter","display_name":"Gap Hunter","phase":3,"file":"gap-hunter.md","memory_keys":["research/analysis/gaps","research/findings/gaps"],"output_artifacts":["research-gaps.md","gap-priorities.md"],"prompt_source_path":".archon/agents/phdresearch/gap-hunter.md","tool_access":["WebSearch","WebFetch","Read","Glob","Grep"]}"###,
    r###"{"key":"risk-analyst","display_name":"Risk Analyst","phase":3,"file":"risk-analyst.md","memory_keys":["research/analysis/risks","research/meta/risks"],"output_artifacts":["risk-assessment.md","risk-mitigation.md"],"prompt_source_path":".archon/agents/phdresearch/risk-analyst.md","tool_access":["WebSearch","WebFetch","Read","Glob","Grep"]}"###,
    r###"{"key":"evidence-synthesizer","display_name":"Evidence Synthesizer","phase":4,"file":"evidence-synthesizer.md","memory_keys":["research/analysis/evidence","research/synthesis/evidence"],"output_artifacts":["evidence-synthesis.md","evidence-matrix.md"],"prompt_source_path":".archon/agents/phdresearch/evidence-synthesizer.md","tool_access":["WebSearch","WebFetch","Read","Glob","Grep"]}"###,
    r###"{"key":"pattern-analyst","display_name":"Pattern Analyst","phase":4,"file":"pattern-analyst.md","memory_keys":["research/synthesis/patterns","research/findings/patterns"],"output_artifacts":["pattern-analysis.md","pattern-catalog.md"],"prompt_source_path":".archon/agents/phdresearch/pattern-analyst.md","tool_access":["WebSearch","WebFetch","Read","Glob","Grep"]}"###,
    r###"{"key":"thematic-synthesizer","display_name":"Thematic Synthesizer","phase":4,"file":"thematic-synthesizer.md","memory_keys":["research/synthesis/themes","research/findings/themes"],"output_artifacts":["thematic-synthesis.md","theme-hierarchy.md"],"prompt_source_path":".archon/agents/phdresearch/thematic-synthesizer.md","tool_access":["WebSearch","WebFetch","Read","Glob","Grep"]}"###,
    r###"{"key":"theory-builder","display_name":"Theory Builder","phase":4,"file":"theory-builder.md","memory_keys":["research/synthesis/theory","research/theory/construction"],"output_artifacts":["theory-development.md","theoretical-model.md"],"prompt_source_path":".archon/agents/phdresearch/theory-builder.md","tool_access":["WebSearch","WebFetch","Read","Glob","Grep"]}"###,
    r###"{"key":"opportunity-identifier","display_name":"Opportunity Identifier","phase":4,"file":"opportunity-identifier.md","memory_keys":["research/synthesis/opportunities","research/findings/opportunities"],"output_artifacts":["research-opportunities.md","opportunity-matrix.md"],"prompt_source_path":".archon/agents/phdresearch/opportunity-identifier.md","tool_access":["WebSearch","WebFetch","Read","Glob","Grep"]}"###,
    r###"{"key":"method-designer","display_name":"Method Designer","phase":5,"file":"method-designer.md","memory_keys":["research/methods/design","research/methodology/approach"],"output_artifacts":["research-design.md","method-rationale.md"],"prompt_source_path":".archon/agents/phdresearch/method-designer.md","tool_access":["WebSearch","WebFetch","Read","Glob","Grep"]}"###,
    r###"{"key":"hypothesis-generator","display_name":"Hypothesis Generator","phase":5,"file":"hypothesis-generator.md","memory_keys":["research/synthesis/hypotheses","research/theory/hypotheses"],"output_artifacts":["hypotheses.md","testable-predictions.md"],"prompt_source_path":".archon/agents/phdresearch/hypothesis-generator.md","tool_access":["WebSearch","WebFetch","Read","Glob","Grep"]}"###,
    r###"{"key":"model-architect","display_name":"Model Architect","phase":5,"file":"model-architect.md","memory_keys":["research/synthesis/models","research/theory/models"],"output_artifacts":["conceptual-model.md","model-specifications.md"],"prompt_source_path":".archon/agents/phdresearch/model-architect.md","tool_access":["WebSearch","WebFetch","Read","Glob","Grep"]}"###,
    r###"{"key":"analysis-planner","display_name":"Analysis Planner","phase":5,"file":"analysis-planner.md","memory_keys":["research/methods/analysis","research/methodology/analysis"],"output_artifacts":["analysis-plan.md","statistical-approach.md"],"prompt_source_path":".archon/agents/phdresearch/analysis-planner.md","tool_access":["WebSearch","WebFetch","Read","Glob","Grep"]}"###,
    r###"{"key":"sampling-strategist","display_name":"Sampling Strategist","phase":5,"file":"sampling-strategist.md","memory_keys":["research/methods/sampling","research/methodology/sampling"],"output_artifacts":["sampling-strategy.md","sample-specifications.md"],"prompt_source_path":".archon/agents/phdresearch/sampling-strategist.md","tool_access":["WebSearch","WebFetch","Read","Glob","Grep"]}"###,
    r###"{"key":"instrument-developer","display_name":"Instrument Developer","phase":5,"file":"instrument-developer.md","memory_keys":["research/methods/instruments","research/methodology/instruments"],"output_artifacts":["research-instruments.md","instrument-validation.md"],"prompt_source_path":".archon/agents/phdresearch/instrument-developer.md","tool_access":["WebSearch","WebFetch","Read","Glob","Grep"]}"###,
    r###"{"key":"validity-guardian","display_name":"Validity Guardian","phase":5,"file":"validity-guardian.md","memory_keys":["research/methods/validity","research/quality/validity"],"output_artifacts":["validity-assessment.md","threat-mitigation.md"],"prompt_source_path":".archon/agents/phdresearch/validity-guardian.md","tool_access":["WebSearch","WebFetch","Read","Glob","Grep"]}"###,
    r###"{"key":"methodology-scanner","display_name":"Methodology Scanner","phase":5,"file":"methodology-scanner.md","memory_keys":["research/literature/methods","research/methodology/survey"],"output_artifacts":["methodology-survey.md","method-comparison.md"],"prompt_source_path":".archon/agents/phdresearch/methodology-scanner.md","tool_access":["WebSearch","WebFetch","Read","Glob","Grep"]}"###,
    r###"{"key":"methodology-writer","display_name":"Methodology Writer","phase":5,"file":"methodology-writer.md","memory_keys":["research/writing/methodology","research/document/chapter3"],"output_artifacts":["methodology-chapter.md","method-details.md"],"prompt_source_path":".archon/agents/phdresearch/methodology-writer.md","tool_access":["WebSearch","WebFetch","Read","Glob","Grep"]}"###,
    r###"{"key":"introduction-writer","display_name":"Introduction Writer","phase":6,"file":"introduction-writer.md","memory_keys":["research/writing/introduction","research/document/chapter1"],"output_artifacts":["introduction.md","problem-statement.md"],"prompt_source_path":".archon/agents/phdresearch/introduction-writer.md","tool_access":["WebSearch","WebFetch","Read","Glob","Grep","Write"]}"###,
    r###"{"key":"literature-review-writer","display_name":"Literature Review Writer","phase":6,"file":"literature-review-writer.md","memory_keys":["research/writing/literature","research/document/chapter2"],"output_artifacts":["literature-review.md","synthesis-narrative.md"],"prompt_source_path":".archon/agents/phdresearch/literature-review-writer.md","tool_access":["WebSearch","WebFetch","Read","Glob","Grep","Write"]}"###,
    r###"{"key":"results-writer","display_name":"Results Writer","phase":6,"file":"results-writer.md","memory_keys":["research/writing/results","research/document/chapter4"],"output_artifacts":["results-chapter.md","findings-narrative.md"],"prompt_source_path":".archon/agents/phdresearch/results-writer.md","tool_access":["WebSearch","WebFetch","Read","Glob","Grep","Write"]}"###,
    r###"{"key":"discussion-writer","display_name":"Discussion Writer","phase":6,"file":"discussion-writer.md","memory_keys":["research/writing/discussion","research/document/chapter5"],"output_artifacts":["discussion-chapter.md","implications.md"],"prompt_source_path":".archon/agents/phdresearch/discussion-writer.md","tool_access":["WebSearch","WebFetch","Read","Glob","Grep","Write"]}"###,
    r###"{"key":"conclusion-writer","display_name":"Conclusion Writer","phase":6,"file":"conclusion-writer.md","memory_keys":["research/writing/conclusion","research/document/chapter6"],"output_artifacts":["conclusion-chapter.md","future-directions.md"],"prompt_source_path":".archon/agents/phdresearch/conclusion-writer.md","tool_access":["WebSearch","WebFetch","Read","Glob","Grep","Write"]}"###,
    r###"{"key":"abstract-writer","display_name":"Abstract Writer","phase":6,"file":"abstract-writer.md","memory_keys":["research/writing/abstract","research/document/abstract"],"output_artifacts":["abstract.md","executive-summary.md"],"prompt_source_path":".archon/agents/phdresearch/abstract-writer.md","tool_access":["WebSearch","WebFetch","Read","Glob","Grep","Write"]}"###,
    r###"{"key":"systematic-reviewer","display_name":"Systematic Reviewer","phase":7,"file":"systematic-reviewer.md","memory_keys":["research/literature/systematic","research/synthesis/systematic-review"],"output_artifacts":["systematic-review.md","prisma-flowchart.md"],"prompt_source_path":".archon/agents/phdresearch/systematic-reviewer.md","tool_access":["WebSearch","WebFetch","Read","Glob","Grep"]}"###,
    r###"{"key":"ethics-reviewer","display_name":"Ethics Reviewer","phase":7,"file":"ethics-reviewer.md","memory_keys":["research/methods/ethics","research/compliance/ethics"],"output_artifacts":["ethics-review.md","irb-protocol.md"],"prompt_source_path":".archon/agents/phdresearch/ethics-reviewer.md","tool_access":["WebSearch","WebFetch","Read","Glob","Grep"]}"###,
    r###"{"key":"adversarial-reviewer","display_name":"Adversarial Reviewer","phase":7,"file":"adversarial-reviewer.md","memory_keys":["research/quality/critique","research/review/adversarial"],"output_artifacts":["adversarial-critique.md","weakness-report.md"],"prompt_source_path":".archon/agents/phdresearch/adversarial-reviewer.md","tool_access":["WebSearch","WebFetch","Read","Glob","Grep"]}"###,
    r###"{"key":"confidence-quantifier","display_name":"Confidence Quantifier","phase":7,"file":"confidence-quantifier.md","memory_keys":["research/quality/confidence","research/meta/certainty"],"output_artifacts":["confidence-scores.md","uncertainty-analysis.md"],"prompt_source_path":".archon/agents/phdresearch/confidence-quantifier.md","tool_access":["WebSearch","WebFetch","Read","Glob","Grep"]}"###,
    r###"{"key":"citation-validator","display_name":"Citation Validator","phase":7,"file":"citation-validator.md","memory_keys":["research/quality/validation","research/sources/verified"],"output_artifacts":["citation-validation.md","source-verification.md"],"prompt_source_path":".archon/agents/phdresearch/citation-validator.md","tool_access":["WebSearch","WebFetch","Read","Glob","Grep"]}"###,
    r###"{"key":"reproducibility-checker","display_name":"Reproducibility Checker","phase":7,"file":"reproducibility-checker.md","memory_keys":["research/quality/reproducibility","research/meta/replication"],"output_artifacts":["reproducibility-report.md","replication-guide.md"],"prompt_source_path":".archon/agents/phdresearch/reproducibility-checker.md","tool_access":["WebSearch","WebFetch","Read","Glob","Grep"]}"###,
    r###"{"key":"apa-citation-specialist","display_name":"APA Citation Specialist","phase":7,"file":"apa-citation-specialist.md","memory_keys":["research/quality/citations","research/document/references"],"output_artifacts":["citation-audit.md","apa-compliance.md"],"prompt_source_path":".archon/agents/phdresearch/apa-citation-specialist.md","tool_access":["WebSearch","WebFetch","Read","Glob","Grep"]}"###,
    r###"{"key":"citation-reconciler","display_name":"Citation Reconciler","phase":7,"file":"citation-reconciler.md","memory_keys":["research/quality/citations","research/document/references","research/quality/citation-repair","research/sources/verified"],"output_artifacts":["citation-reconciliation.md","master-reference-list.md"],"prompt_source_path":".archon/agents/phdresearch/citation-reconciler.md","tool_access":["WebSearch","WebFetch","Read","Glob","Grep"]}"###,
    r###"{"key":"consistency-validator","display_name":"Consistency Validator","phase":7,"file":"consistency-validator.md","memory_keys":["research/quality/consistency","research/document/coherence"],"output_artifacts":["consistency-report.md","coherence-audit.md"],"prompt_source_path":".archon/agents/phdresearch/consistency-validator.md","tool_access":["WebSearch","WebFetch","Read","Glob","Grep"]}"###,
    r###"{"key":"quality-assessor","display_name":"Quality Assessor","phase":7,"file":"quality-assessor.md","memory_keys":["research/analysis/quality","research/meta/assessment"],"output_artifacts":["quality-assessment.md","quality-scores.md"],"prompt_source_path":".archon/agents/phdresearch/quality-assessor.md","tool_access":["WebSearch","WebFetch","Read","Glob","Grep"]}"###,
    r###"{"key":"bias-detector","display_name":"Bias Detector","phase":7,"file":"bias-detector.md","memory_keys":["research/analysis/bias","research/quality/bias"],"output_artifacts":["bias-analysis.md","bias-mitigation.md"],"prompt_source_path":".archon/agents/phdresearch/bias-detector.md","tool_access":["WebSearch","WebFetch","Read","Glob","Grep"]}"###,
    r###"{"key":"file-length-manager","display_name":"File Length Manager","phase":7,"file":"file-length-manager.md","memory_keys":["research/quality/structure","research/document/formatting"],"output_artifacts":["structure-audit.md","length-compliance.md"],"prompt_source_path":".archon/agents/phdresearch/file-length-manager.md","tool_access":["WebSearch","WebFetch","Read","Glob","Grep"]}"###,
    r###"{"key":"chapter-synthesizer","display_name":"Chapter Synthesizer","phase":8,"file":"chapter-synthesizer.md","memory_keys":["research/document/final","research/structure/chapters","research/writing/introduction","research/writing/literature","research/writing/methodology","research/writing/results","research/writing/discussion","research/writing/conclusion","research/writing/abstract","research/quality/citations","research/quality/validation","research/quality/citation-repair","research/document/references","research/quality/consistency","research/quality/structure"],"output_artifacts":["final-paper.md","dissertation-complete.md"],"prompt_source_path":".archon/agents/phdresearch/chapter-synthesizer.md","tool_access":["WebSearch","WebFetch","Read","Glob","Grep","Write"]}"###,
];
const EXPECTED_RESEARCH_PHASE_JSON: &[&str] = &[
    r###"{"id":1,"name":"Foundation","description":"Initial problem analysis, step-back reasoning, question decomposition, ambiguity resolution, research planning, construct definition, dissertation architecture, and chapter synthesis framework.","agent_keys":["step-back-analyzer","self-ask-decomposer","ambiguity-clarifier","research-planner","construct-definer","dissertation-architect"]}"###,
    r###"{"id":2,"name":"Discovery","description":"Comprehensive literature mapping, source classification by credibility tiers, citation extraction, and context tier management.","agent_keys":["literature-mapper","source-tier-classifier","citation-extractor","context-tier-manager"]}"###,
    r###"{"id":3,"name":"Architecture","description":"Theoretical framework analysis, contradiction detection, gap identification, and risk assessment.","agent_keys":["theoretical-framework-analyst","contradiction-analyzer","gap-hunter","risk-analyst"]}"###,
    r###"{"id":4,"name":"Synthesis","description":"Evidence synthesis, pattern recognition, thematic synthesis, theory building, and opportunity identification.","agent_keys":["evidence-synthesizer","pattern-analyst","thematic-synthesizer","theory-builder","opportunity-identifier"]}"###,
    r###"{"id":5,"name":"Design","description":"Research methodology design, hypothesis generation, model architecture, analysis planning, sampling strategy, instrument development, validity assurance, methodology scanning, and methodology writing.","agent_keys":["method-designer","hypothesis-generator","model-architect","analysis-planner","sampling-strategist","instrument-developer","validity-guardian","methodology-scanner","methodology-writer"]}"###,
    r###"{"id":6,"name":"Writing","description":"Document creation including introduction, literature review, results, discussion, conclusion, and abstract chapters.","agent_keys":["introduction-writer","literature-review-writer","results-writer","discussion-writer","conclusion-writer","abstract-writer"]}"###,
    r###"{"id":7,"name":"Validation","description":"Final quality assurance including systematic review, ethics review, adversarial review, confidence quantification, citation validation, reproducibility checking, APA formatting, citation reconciliation, consistency validation, quality assessment, bias detection, and file length management.","agent_keys":["systematic-reviewer","ethics-reviewer","adversarial-reviewer","confidence-quantifier","citation-validator","reproducibility-checker","apa-citation-specialist","citation-reconciler","consistency-validator","quality-assessor","bias-detector","file-length-manager"]}"###,
    r###"{"id":8,"name":"Final Assembly","description":"Compose the validated chapter outputs, citation audit, and structural checks into the final university-standard research paper.","agent_keys":["chapter-synthesizer"]}"###,
];
const EXPECTED_RESEARCH_PHASE_KEYS: &[&[&str]] = &[
    &[
        r###"step-back-analyzer"###,
        r###"self-ask-decomposer"###,
        r###"ambiguity-clarifier"###,
        r###"research-planner"###,
        r###"construct-definer"###,
        r###"dissertation-architect"###,
    ],
    &[
        r###"literature-mapper"###,
        r###"source-tier-classifier"###,
        r###"citation-extractor"###,
        r###"context-tier-manager"###,
    ],
    &[
        r###"theoretical-framework-analyst"###,
        r###"contradiction-analyzer"###,
        r###"gap-hunter"###,
        r###"risk-analyst"###,
    ],
    &[
        r###"evidence-synthesizer"###,
        r###"pattern-analyst"###,
        r###"thematic-synthesizer"###,
        r###"theory-builder"###,
        r###"opportunity-identifier"###,
    ],
    &[
        r###"method-designer"###,
        r###"hypothesis-generator"###,
        r###"model-architect"###,
        r###"analysis-planner"###,
        r###"sampling-strategist"###,
        r###"instrument-developer"###,
        r###"validity-guardian"###,
        r###"methodology-scanner"###,
        r###"methodology-writer"###,
    ],
    &[
        r###"introduction-writer"###,
        r###"literature-review-writer"###,
        r###"results-writer"###,
        r###"discussion-writer"###,
        r###"conclusion-writer"###,
        r###"abstract-writer"###,
    ],
    &[
        r###"systematic-reviewer"###,
        r###"ethics-reviewer"###,
        r###"adversarial-reviewer"###,
        r###"confidence-quantifier"###,
        r###"citation-validator"###,
        r###"reproducibility-checker"###,
        r###"apa-citation-specialist"###,
        r###"citation-reconciler"###,
        r###"consistency-validator"###,
        r###"quality-assessor"###,
        r###"bias-detector"###,
        r###"file-length-manager"###,
    ],
    &[r###"chapter-synthesizer"###],
];
#[test]
fn research_definitions_match_pre_split_baseline() {
    let keys = RESEARCH_AGENTS
        .iter()
        .map(|agent| agent.key)
        .collect::<Vec<_>>();
    assert_eq!(keys.as_slice(), EXPECTED_RESEARCH_KEYS);
    let serialized = RESEARCH_AGENTS
        .iter()
        .map(|agent| serde_json::to_string(agent).expect("serialize research agent"))
        .collect::<Vec<_>>();
    assert_eq!(serialized.as_slice(), EXPECTED_RESEARCH_JSON);
}
#[test]
fn research_phase_definitions_match_pre_split_baseline() {
    let phase_json = RESEARCH_PHASES
        .iter()
        .map(|phase| serde_json::to_string(phase).expect("serialize research phase"))
        .collect::<Vec<_>>();
    assert_eq!(phase_json.as_slice(), EXPECTED_RESEARCH_PHASE_JSON);
    assert_eq!(
        RESEARCH_PHASES.len(),
        EXPECTED_RESEARCH_PHASE_KEYS.len(),
        "expected and runtime research phase counts must match",
    );
    for (phase, expected_keys) in RESEARCH_PHASES.iter().zip(EXPECTED_RESEARCH_PHASE_KEYS) {
        assert_eq!(phase.agent_keys, *expected_keys);
    }
}
#[test]
fn research_phase_membership_and_lookup_order_match_pre_split_baseline() {
    let phase_keys = RESEARCH_PHASES
        .iter()
        .flat_map(|phase| phase.agent_keys.iter().copied())
        .collect::<Vec<_>>();
    assert_eq!(phase_keys.as_slice(), EXPECTED_RESEARCH_KEYS);
    for (index, key) in EXPECTED_RESEARCH_KEYS.iter().enumerate() {
        assert_eq!(get_agent_index(key), Some(index));
        assert_eq!(RESEARCH_AGENTS[index].key, *key);
        if index > 0 {
            assert_eq!(
                RESEARCH_AGENTS[index - 1].key,
                EXPECTED_RESEARCH_KEYS[index - 1]
            );
        }
        if index + 1 < EXPECTED_RESEARCH_KEYS.len() {
            assert_eq!(
                RESEARCH_AGENTS[index + 1].key,
                EXPECTED_RESEARCH_KEYS[index + 1]
            );
        }
    }
}
