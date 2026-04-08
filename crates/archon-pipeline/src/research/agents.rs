//! 46-agent research pipeline definitions.
//!
//! Ports the TypeScript PhD pipeline agent configuration to Rust.
//! 46 agents across 7 phases:
//!
//! - Phase 1 Foundation (7): step-back analysis, decomposition, planning, architecture
//! - Phase 2 Discovery (4): literature mapping, source classification, citations
//! - Phase 3 Architecture (4): theoretical framework, contradictions, gaps, risks
//! - Phase 4 Synthesis (5): evidence synthesis, patterns, themes, theory building
//! - Phase 5 Design (9): methodology, hypotheses, models, instruments, validity
//! - Phase 6 Writing (6): dissertation chapter writing (introduction through abstract)
//! - Phase 7 Validation (11): systematic review, ethics, quality assurance

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Tool access capabilities for research agents.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ResearchToolAccess {
    WebSearch,
    WebFetch,
    Read,
    Glob,
    Grep,
    Write,
}

/// Base tool set for all research agents (no Write).
const BASE_TOOLS: &[ResearchToolAccess] = &[
    ResearchToolAccess::WebSearch,
    ResearchToolAccess::WebFetch,
    ResearchToolAccess::Read,
    ResearchToolAccess::Glob,
    ResearchToolAccess::Grep,
];

/// Extended tool set for Phase 6 writing agents (includes Write).
const WRITER_TOOLS: &[ResearchToolAccess] = &[
    ResearchToolAccess::WebSearch,
    ResearchToolAccess::WebFetch,
    ResearchToolAccess::Read,
    ResearchToolAccess::Glob,
    ResearchToolAccess::Grep,
    ResearchToolAccess::Write,
];

/// A single research pipeline agent definition.
#[derive(Clone, Debug, Serialize)]
pub struct ResearchAgent {
    pub key: &'static str,
    pub display_name: &'static str,
    pub phase: u8,
    pub file: &'static str,
    #[serde(serialize_with = "ser_static_str_slice")]
    pub memory_keys: &'static [&'static str],
    #[serde(serialize_with = "ser_static_str_slice")]
    pub output_artifacts: &'static [&'static str],
    pub prompt_source_path: &'static str,
    #[serde(serialize_with = "ser_tool_access_slice")]
    pub tool_access: &'static [ResearchToolAccess],
}

/// A research pipeline phase definition.
#[derive(Clone, Debug, Serialize)]
pub struct ResearchPhase {
    pub id: u8,
    pub name: &'static str,
    pub description: &'static str,
    #[serde(serialize_with = "ser_static_str_slice")]
    pub agent_keys: &'static [&'static str],
}

// ---------------------------------------------------------------------------
// Serde helpers
// ---------------------------------------------------------------------------

fn ser_static_str_slice<S: serde::Serializer>(
    v: &&'static [&'static str],
    s: S,
) -> Result<S::Ok, S::Error> {
    use serde::ser::SerializeSeq;
    let mut seq = s.serialize_seq(Some(v.len()))?;
    for item in *v {
        seq.serialize_element(item)?;
    }
    seq.end()
}

fn ser_tool_access_slice<S: serde::Serializer>(
    v: &&'static [ResearchToolAccess],
    s: S,
) -> Result<S::Ok, S::Error> {
    use serde::ser::SerializeSeq;
    let mut seq = s.serialize_seq(Some(v.len()))?;
    for item in *v {
        seq.serialize_element(item)?;
    }
    seq.end()
}

/// Owned mirror of [`ResearchAgent`] used exclusively for deserialization.
#[derive(Deserialize)]
struct OwnedResearchAgent {
    key: String,
    display_name: String,
    phase: u8,
    file: String,
    memory_keys: Vec<String>,
    output_artifacts: Vec<String>,
    prompt_source_path: String,
    tool_access: Vec<ResearchToolAccess>,
}

impl<'de> Deserialize<'de> for ResearchAgent {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let owned = OwnedResearchAgent::deserialize(deserializer)?;
        Ok(ResearchAgent {
            key: Box::leak(owned.key.into_boxed_str()),
            display_name: Box::leak(owned.display_name.into_boxed_str()),
            phase: owned.phase,
            file: Box::leak(owned.file.into_boxed_str()),
            memory_keys: Box::leak(
                owned
                    .memory_keys
                    .into_iter()
                    .map(|s| &*Box::leak(s.into_boxed_str()))
                    .collect::<Vec<&'static str>>()
                    .into_boxed_slice(),
            ),
            output_artifacts: Box::leak(
                owned
                    .output_artifacts
                    .into_iter()
                    .map(|s| &*Box::leak(s.into_boxed_str()))
                    .collect::<Vec<&'static str>>()
                    .into_boxed_slice(),
            ),
            prompt_source_path: Box::leak(owned.prompt_source_path.into_boxed_str()),
            tool_access: Box::leak(owned.tool_access.into_boxed_slice()),
        })
    }
}

/// Owned mirror of [`ResearchPhase`] used exclusively for deserialization.
#[derive(Deserialize)]
struct OwnedResearchPhase {
    id: u8,
    name: String,
    description: String,
    agent_keys: Vec<String>,
}

impl<'de> Deserialize<'de> for ResearchPhase {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let owned = OwnedResearchPhase::deserialize(deserializer)?;
        Ok(ResearchPhase {
            id: owned.id,
            name: Box::leak(owned.name.into_boxed_str()),
            description: Box::leak(owned.description.into_boxed_str()),
            agent_keys: Box::leak(
                owned
                    .agent_keys
                    .into_iter()
                    .map(|s| &*Box::leak(s.into_boxed_str()))
                    .collect::<Vec<&'static str>>()
                    .into_boxed_slice(),
            ),
        })
    }
}

// ---------------------------------------------------------------------------
// 46 agent definitions
// ---------------------------------------------------------------------------

/// All 46 research-pipeline agents in execution order.
pub static RESEARCH_AGENTS: &[ResearchAgent] = &[
    // =========================================================================
    // PHASE 1: FOUNDATION (7 agents, indices 0-6)
    // =========================================================================
    ResearchAgent {
        key: "step-back-analyzer",
        display_name: "Step-Back Analyzer",
        phase: 1,
        file: "step-back-analyzer.md",
        memory_keys: &["research/foundation/framing", "research/meta/perspective"],
        output_artifacts: &["high-level-framing.md", "abstraction-analysis.md"],
        prompt_source_path: ".archon/agents/phdresearch/step-back-analyzer.md",
        tool_access: BASE_TOOLS,
    },
    ResearchAgent {
        key: "self-ask-decomposer",
        display_name: "Self-Ask Decomposer",
        phase: 1,
        file: "self-ask-decomposer.md",
        memory_keys: &[
            "research/meta/questions",
            "research/foundation/decomposition",
        ],
        output_artifacts: &["essential-questions.md", "knowledge-gaps.md"],
        prompt_source_path: ".archon/agents/phdresearch/self-ask-decomposer.md",
        tool_access: BASE_TOOLS,
    },
    ResearchAgent {
        key: "ambiguity-clarifier",
        display_name: "Ambiguity Clarifier",
        phase: 1,
        file: "ambiguity-clarifier.md",
        memory_keys: &[
            "research/foundation/definitions",
            "research/meta/clarifications",
        ],
        output_artifacts: &["term-definitions.md", "clarified-scope.md"],
        prompt_source_path: ".archon/agents/phdresearch/ambiguity-clarifier.md",
        tool_access: BASE_TOOLS,
    },
    ResearchAgent {
        key: "research-planner",
        display_name: "Research Planner",
        phase: 1,
        file: "research-planner.md",
        memory_keys: &["research/foundation/plan", "research/meta/strategy"],
        output_artifacts: &["research-plan.md", "timeline.md"],
        prompt_source_path: ".archon/agents/phdresearch/research-planner.md",
        tool_access: BASE_TOOLS,
    },
    ResearchAgent {
        key: "construct-definer",
        display_name: "Construct Definer",
        phase: 1,
        file: "construct-definer.md",
        memory_keys: &[
            "research/foundation/constructs",
            "research/theory/definitions",
        ],
        output_artifacts: &["construct-definitions.md", "operationalizations.md"],
        prompt_source_path: ".archon/agents/phdresearch/construct-definer.md",
        tool_access: BASE_TOOLS,
    },
    ResearchAgent {
        key: "dissertation-architect",
        display_name: "Dissertation Architect",
        phase: 1,
        file: "dissertation-architect.md",
        memory_keys: &[
            "research/writing/structure",
            "research/document/architecture",
        ],
        output_artifacts: &["dissertation-outline.md", "chapter-structure.md"],
        prompt_source_path: ".archon/agents/phdresearch/dissertation-architect.md",
        tool_access: BASE_TOOLS,
    },
    ResearchAgent {
        key: "chapter-synthesizer",
        display_name: "Chapter Synthesizer",
        phase: 1,
        file: "chapter-synthesizer.md",
        memory_keys: &["research/quality/synthesis", "research/document/final"],
        output_artifacts: &["final-synthesis.md", "dissertation-complete.md"],
        prompt_source_path: ".archon/agents/phdresearch/chapter-synthesizer.md",
        tool_access: BASE_TOOLS,
    },
    // =========================================================================
    // PHASE 2: DISCOVERY (4 agents, indices 7-10)
    // =========================================================================
    ResearchAgent {
        key: "literature-mapper",
        display_name: "Literature Mapper",
        phase: 2,
        file: "literature-mapper.md",
        memory_keys: &["research/literature/map", "research/sources/index"],
        output_artifacts: &["literature-map.md", "source-catalog.md"],
        prompt_source_path: ".archon/agents/phdresearch/literature-mapper.md",
        tool_access: BASE_TOOLS,
    },
    ResearchAgent {
        key: "source-tier-classifier",
        display_name: "Source Tier Classifier",
        phase: 2,
        file: "source-tier-classifier.md",
        memory_keys: &["research/literature/tiers", "research/quality/sources"],
        output_artifacts: &["source-tiers.md", "credibility-assessment.md"],
        prompt_source_path: ".archon/agents/phdresearch/source-tier-classifier.md",
        tool_access: BASE_TOOLS,
    },
    ResearchAgent {
        key: "citation-extractor",
        display_name: "Citation Extractor",
        phase: 2,
        file: "citation-extractor.md",
        memory_keys: &["research/quality/extraction", "research/sources/citations"],
        output_artifacts: &["extracted-citations.md", "reference-list.md"],
        prompt_source_path: ".archon/agents/phdresearch/citation-extractor.md",
        tool_access: BASE_TOOLS,
    },
    ResearchAgent {
        key: "context-tier-manager",
        display_name: "Context Tier Manager",
        phase: 2,
        file: "context-tier-manager.md",
        memory_keys: &["research/literature/context", "research/meta/tiers"],
        output_artifacts: &["context-hierarchy.md", "tier-mappings.md"],
        prompt_source_path: ".archon/agents/phdresearch/context-tier-manager.md",
        tool_access: BASE_TOOLS,
    },
    // =========================================================================
    // PHASE 3: ARCHITECTURE (4 agents, indices 11-14)
    // =========================================================================
    ResearchAgent {
        key: "theoretical-framework-analyst",
        display_name: "Theoretical Framework Analyst",
        phase: 3,
        file: "theoretical-framework-analyst.md",
        memory_keys: &["research/foundation/framework", "research/theory/analysis"],
        output_artifacts: &["theoretical-framework.md", "framework-map.md"],
        prompt_source_path: ".archon/agents/phdresearch/theoretical-framework-analyst.md",
        tool_access: BASE_TOOLS,
    },
    ResearchAgent {
        key: "contradiction-analyzer",
        display_name: "Contradiction Analyzer",
        phase: 3,
        file: "contradiction-analyzer.md",
        memory_keys: &[
            "research/analysis/contradictions",
            "research/findings/conflicts",
        ],
        output_artifacts: &["contradictions-report.md", "resolution-proposals.md"],
        prompt_source_path: ".archon/agents/phdresearch/contradiction-analyzer.md",
        tool_access: BASE_TOOLS,
    },
    ResearchAgent {
        key: "gap-hunter",
        display_name: "Gap Hunter",
        phase: 3,
        file: "gap-hunter.md",
        memory_keys: &["research/analysis/gaps", "research/findings/gaps"],
        output_artifacts: &["research-gaps.md", "gap-priorities.md"],
        prompt_source_path: ".archon/agents/phdresearch/gap-hunter.md",
        tool_access: BASE_TOOLS,
    },
    ResearchAgent {
        key: "risk-analyst",
        display_name: "Risk Analyst",
        phase: 3,
        file: "risk-analyst.md",
        memory_keys: &["research/analysis/risks", "research/meta/risks"],
        output_artifacts: &["risk-assessment.md", "risk-mitigation.md"],
        prompt_source_path: ".archon/agents/phdresearch/risk-analyst.md",
        tool_access: BASE_TOOLS,
    },
    // =========================================================================
    // PHASE 4: SYNTHESIS (5 agents, indices 15-19)
    // =========================================================================
    ResearchAgent {
        key: "evidence-synthesizer",
        display_name: "Evidence Synthesizer",
        phase: 4,
        file: "evidence-synthesizer.md",
        memory_keys: &["research/analysis/evidence", "research/synthesis/evidence"],
        output_artifacts: &["evidence-synthesis.md", "evidence-matrix.md"],
        prompt_source_path: ".archon/agents/phdresearch/evidence-synthesizer.md",
        tool_access: BASE_TOOLS,
    },
    ResearchAgent {
        key: "pattern-analyst",
        display_name: "Pattern Analyst",
        phase: 4,
        file: "pattern-analyst.md",
        memory_keys: &["research/synthesis/patterns", "research/findings/patterns"],
        output_artifacts: &["pattern-analysis.md", "pattern-catalog.md"],
        prompt_source_path: ".archon/agents/phdresearch/pattern-analyst.md",
        tool_access: BASE_TOOLS,
    },
    ResearchAgent {
        key: "thematic-synthesizer",
        display_name: "Thematic Synthesizer",
        phase: 4,
        file: "thematic-synthesizer.md",
        memory_keys: &["research/synthesis/themes", "research/findings/themes"],
        output_artifacts: &["thematic-synthesis.md", "theme-hierarchy.md"],
        prompt_source_path: ".archon/agents/phdresearch/thematic-synthesizer.md",
        tool_access: BASE_TOOLS,
    },
    ResearchAgent {
        key: "theory-builder",
        display_name: "Theory Builder",
        phase: 4,
        file: "theory-builder.md",
        memory_keys: &["research/synthesis/theory", "research/theory/construction"],
        output_artifacts: &["theory-development.md", "theoretical-model.md"],
        prompt_source_path: ".archon/agents/phdresearch/theory-builder.md",
        tool_access: BASE_TOOLS,
    },
    ResearchAgent {
        key: "opportunity-identifier",
        display_name: "Opportunity Identifier",
        phase: 4,
        file: "opportunity-identifier.md",
        memory_keys: &[
            "research/synthesis/opportunities",
            "research/findings/opportunities",
        ],
        output_artifacts: &["research-opportunities.md", "opportunity-matrix.md"],
        prompt_source_path: ".archon/agents/phdresearch/opportunity-identifier.md",
        tool_access: BASE_TOOLS,
    },
    // =========================================================================
    // PHASE 5: DESIGN (9 agents, indices 20-28)
    // =========================================================================
    ResearchAgent {
        key: "method-designer",
        display_name: "Method Designer",
        phase: 5,
        file: "method-designer.md",
        memory_keys: &["research/methods/design", "research/methodology/approach"],
        output_artifacts: &["research-design.md", "method-rationale.md"],
        prompt_source_path: ".archon/agents/phdresearch/method-designer.md",
        tool_access: BASE_TOOLS,
    },
    ResearchAgent {
        key: "hypothesis-generator",
        display_name: "Hypothesis Generator",
        phase: 5,
        file: "hypothesis-generator.md",
        memory_keys: &[
            "research/synthesis/hypotheses",
            "research/theory/hypotheses",
        ],
        output_artifacts: &["hypotheses.md", "testable-predictions.md"],
        prompt_source_path: ".archon/agents/phdresearch/hypothesis-generator.md",
        tool_access: BASE_TOOLS,
    },
    ResearchAgent {
        key: "model-architect",
        display_name: "Model Architect",
        phase: 5,
        file: "model-architect.md",
        memory_keys: &["research/synthesis/models", "research/theory/models"],
        output_artifacts: &["conceptual-model.md", "model-specifications.md"],
        prompt_source_path: ".archon/agents/phdresearch/model-architect.md",
        tool_access: BASE_TOOLS,
    },
    ResearchAgent {
        key: "analysis-planner",
        display_name: "Analysis Planner",
        phase: 5,
        file: "analysis-planner.md",
        memory_keys: &["research/methods/analysis", "research/methodology/analysis"],
        output_artifacts: &["analysis-plan.md", "statistical-approach.md"],
        prompt_source_path: ".archon/agents/phdresearch/analysis-planner.md",
        tool_access: BASE_TOOLS,
    },
    ResearchAgent {
        key: "sampling-strategist",
        display_name: "Sampling Strategist",
        phase: 5,
        file: "sampling-strategist.md",
        memory_keys: &["research/methods/sampling", "research/methodology/sampling"],
        output_artifacts: &["sampling-strategy.md", "sample-specifications.md"],
        prompt_source_path: ".archon/agents/phdresearch/sampling-strategist.md",
        tool_access: BASE_TOOLS,
    },
    ResearchAgent {
        key: "instrument-developer",
        display_name: "Instrument Developer",
        phase: 5,
        file: "instrument-developer.md",
        memory_keys: &[
            "research/methods/instruments",
            "research/methodology/instruments",
        ],
        output_artifacts: &["research-instruments.md", "instrument-validation.md"],
        prompt_source_path: ".archon/agents/phdresearch/instrument-developer.md",
        tool_access: BASE_TOOLS,
    },
    ResearchAgent {
        key: "validity-guardian",
        display_name: "Validity Guardian",
        phase: 5,
        file: "validity-guardian.md",
        memory_keys: &["research/methods/validity", "research/quality/validity"],
        output_artifacts: &["validity-assessment.md", "threat-mitigation.md"],
        prompt_source_path: ".archon/agents/phdresearch/validity-guardian.md",
        tool_access: BASE_TOOLS,
    },
    ResearchAgent {
        key: "methodology-scanner",
        display_name: "Methodology Scanner",
        phase: 5,
        file: "methodology-scanner.md",
        memory_keys: &["research/literature/methods", "research/methodology/survey"],
        output_artifacts: &["methodology-survey.md", "method-comparison.md"],
        prompt_source_path: ".archon/agents/phdresearch/methodology-scanner.md",
        tool_access: BASE_TOOLS,
    },
    ResearchAgent {
        key: "methodology-writer",
        display_name: "Methodology Writer",
        phase: 5,
        file: "methodology-writer.md",
        memory_keys: &["research/writing/methodology", "research/document/chapter3"],
        output_artifacts: &["methodology-chapter.md", "method-details.md"],
        prompt_source_path: ".archon/agents/phdresearch/methodology-writer.md",
        tool_access: BASE_TOOLS,
    },
    // =========================================================================
    // PHASE 6: WRITING (6 agents, indices 29-34)
    // =========================================================================
    ResearchAgent {
        key: "introduction-writer",
        display_name: "Introduction Writer",
        phase: 6,
        file: "introduction-writer.md",
        memory_keys: &[
            "research/writing/introduction",
            "research/document/chapter1",
        ],
        output_artifacts: &["introduction.md", "problem-statement.md"],
        prompt_source_path: ".archon/agents/phdresearch/introduction-writer.md",
        tool_access: WRITER_TOOLS,
    },
    ResearchAgent {
        key: "literature-review-writer",
        display_name: "Literature Review Writer",
        phase: 6,
        file: "literature-review-writer.md",
        memory_keys: &["research/writing/literature", "research/document/chapter2"],
        output_artifacts: &["literature-review.md", "synthesis-narrative.md"],
        prompt_source_path: ".archon/agents/phdresearch/literature-review-writer.md",
        tool_access: WRITER_TOOLS,
    },
    ResearchAgent {
        key: "results-writer",
        display_name: "Results Writer",
        phase: 6,
        file: "results-writer.md",
        memory_keys: &["research/writing/results", "research/document/chapter4"],
        output_artifacts: &["results-chapter.md", "findings-narrative.md"],
        prompt_source_path: ".archon/agents/phdresearch/results-writer.md",
        tool_access: WRITER_TOOLS,
    },
    ResearchAgent {
        key: "discussion-writer",
        display_name: "Discussion Writer",
        phase: 6,
        file: "discussion-writer.md",
        memory_keys: &["research/writing/discussion", "research/document/chapter5"],
        output_artifacts: &["discussion-chapter.md", "implications.md"],
        prompt_source_path: ".archon/agents/phdresearch/discussion-writer.md",
        tool_access: WRITER_TOOLS,
    },
    ResearchAgent {
        key: "conclusion-writer",
        display_name: "Conclusion Writer",
        phase: 6,
        file: "conclusion-writer.md",
        memory_keys: &["research/writing/conclusion", "research/document/chapter6"],
        output_artifacts: &["conclusion-chapter.md", "future-directions.md"],
        prompt_source_path: ".archon/agents/phdresearch/conclusion-writer.md",
        tool_access: WRITER_TOOLS,
    },
    ResearchAgent {
        key: "abstract-writer",
        display_name: "Abstract Writer",
        phase: 6,
        file: "abstract-writer.md",
        memory_keys: &["research/writing/abstract", "research/document/abstract"],
        output_artifacts: &["abstract.md", "executive-summary.md"],
        prompt_source_path: ".archon/agents/phdresearch/abstract-writer.md",
        tool_access: WRITER_TOOLS,
    },
    // =========================================================================
    // PHASE 7: VALIDATION (11 agents, indices 35-45)
    // =========================================================================
    ResearchAgent {
        key: "systematic-reviewer",
        display_name: "Systematic Reviewer",
        phase: 7,
        file: "systematic-reviewer.md",
        memory_keys: &[
            "research/literature/systematic",
            "research/synthesis/systematic-review",
        ],
        output_artifacts: &["systematic-review.md", "prisma-flowchart.md"],
        prompt_source_path: ".archon/agents/phdresearch/systematic-reviewer.md",
        tool_access: BASE_TOOLS,
    },
    ResearchAgent {
        key: "ethics-reviewer",
        display_name: "Ethics Reviewer",
        phase: 7,
        file: "ethics-reviewer.md",
        memory_keys: &["research/methods/ethics", "research/compliance/ethics"],
        output_artifacts: &["ethics-review.md", "irb-protocol.md"],
        prompt_source_path: ".archon/agents/phdresearch/ethics-reviewer.md",
        tool_access: BASE_TOOLS,
    },
    ResearchAgent {
        key: "adversarial-reviewer",
        display_name: "Adversarial Reviewer",
        phase: 7,
        file: "adversarial-reviewer.md",
        memory_keys: &["research/quality/critique", "research/review/adversarial"],
        output_artifacts: &["adversarial-critique.md", "weakness-report.md"],
        prompt_source_path: ".archon/agents/phdresearch/adversarial-reviewer.md",
        tool_access: BASE_TOOLS,
    },
    ResearchAgent {
        key: "confidence-quantifier",
        display_name: "Confidence Quantifier",
        phase: 7,
        file: "confidence-quantifier.md",
        memory_keys: &["research/quality/confidence", "research/meta/certainty"],
        output_artifacts: &["confidence-scores.md", "uncertainty-analysis.md"],
        prompt_source_path: ".archon/agents/phdresearch/confidence-quantifier.md",
        tool_access: BASE_TOOLS,
    },
    ResearchAgent {
        key: "citation-validator",
        display_name: "Citation Validator",
        phase: 7,
        file: "citation-validator.md",
        memory_keys: &["research/quality/validation", "research/sources/verified"],
        output_artifacts: &["citation-validation.md", "source-verification.md"],
        prompt_source_path: ".archon/agents/phdresearch/citation-validator.md",
        tool_access: BASE_TOOLS,
    },
    ResearchAgent {
        key: "reproducibility-checker",
        display_name: "Reproducibility Checker",
        phase: 7,
        file: "reproducibility-checker.md",
        memory_keys: &[
            "research/quality/reproducibility",
            "research/meta/replication",
        ],
        output_artifacts: &["reproducibility-report.md", "replication-guide.md"],
        prompt_source_path: ".archon/agents/phdresearch/reproducibility-checker.md",
        tool_access: BASE_TOOLS,
    },
    ResearchAgent {
        key: "apa-citation-specialist",
        display_name: "APA Citation Specialist",
        phase: 7,
        file: "apa-citation-specialist.md",
        memory_keys: &["research/quality/citations", "research/document/references"],
        output_artifacts: &["citation-audit.md", "apa-compliance.md"],
        prompt_source_path: ".archon/agents/phdresearch/apa-citation-specialist.md",
        tool_access: BASE_TOOLS,
    },
    ResearchAgent {
        key: "consistency-validator",
        display_name: "Consistency Validator",
        phase: 7,
        file: "consistency-validator.md",
        memory_keys: &[
            "research/quality/consistency",
            "research/document/coherence",
        ],
        output_artifacts: &["consistency-report.md", "coherence-audit.md"],
        prompt_source_path: ".archon/agents/phdresearch/consistency-validator.md",
        tool_access: BASE_TOOLS,
    },
    ResearchAgent {
        key: "quality-assessor",
        display_name: "Quality Assessor",
        phase: 7,
        file: "quality-assessor.md",
        memory_keys: &["research/analysis/quality", "research/meta/assessment"],
        output_artifacts: &["quality-assessment.md", "quality-scores.md"],
        prompt_source_path: ".archon/agents/phdresearch/quality-assessor.md",
        tool_access: BASE_TOOLS,
    },
    ResearchAgent {
        key: "bias-detector",
        display_name: "Bias Detector",
        phase: 7,
        file: "bias-detector.md",
        memory_keys: &["research/analysis/bias", "research/quality/bias"],
        output_artifacts: &["bias-analysis.md", "bias-mitigation.md"],
        prompt_source_path: ".archon/agents/phdresearch/bias-detector.md",
        tool_access: BASE_TOOLS,
    },
    ResearchAgent {
        key: "file-length-manager",
        display_name: "File Length Manager",
        phase: 7,
        file: "file-length-manager.md",
        memory_keys: &["research/quality/structure", "research/document/formatting"],
        output_artifacts: &["structure-audit.md", "length-compliance.md"],
        prompt_source_path: ".archon/agents/phdresearch/file-length-manager.md",
        tool_access: BASE_TOOLS,
    },
];

// ---------------------------------------------------------------------------
// 7 phase definitions
// ---------------------------------------------------------------------------

/// All 7 research-pipeline phases in order.
pub static RESEARCH_PHASES: &[ResearchPhase] = &[
    ResearchPhase {
        id: 1,
        name: "Foundation",
        description: "Initial problem analysis, step-back reasoning, question decomposition, ambiguity resolution, research planning, construct definition, dissertation architecture, and chapter synthesis framework.",
        agent_keys: &[
            "step-back-analyzer",
            "self-ask-decomposer",
            "ambiguity-clarifier",
            "research-planner",
            "construct-definer",
            "dissertation-architect",
            "chapter-synthesizer",
        ],
    },
    ResearchPhase {
        id: 2,
        name: "Discovery",
        description: "Comprehensive literature mapping, source classification by credibility tiers, citation extraction, and context tier management.",
        agent_keys: &[
            "literature-mapper",
            "source-tier-classifier",
            "citation-extractor",
            "context-tier-manager",
        ],
    },
    ResearchPhase {
        id: 3,
        name: "Architecture",
        description: "Theoretical framework analysis, contradiction detection, gap identification, and risk assessment.",
        agent_keys: &[
            "theoretical-framework-analyst",
            "contradiction-analyzer",
            "gap-hunter",
            "risk-analyst",
        ],
    },
    ResearchPhase {
        id: 4,
        name: "Synthesis",
        description: "Evidence synthesis, pattern recognition, thematic synthesis, theory building, and opportunity identification.",
        agent_keys: &[
            "evidence-synthesizer",
            "pattern-analyst",
            "thematic-synthesizer",
            "theory-builder",
            "opportunity-identifier",
        ],
    },
    ResearchPhase {
        id: 5,
        name: "Design",
        description: "Research methodology design, hypothesis generation, model architecture, analysis planning, sampling strategy, instrument development, validity assurance, methodology scanning, and methodology writing.",
        agent_keys: &[
            "method-designer",
            "hypothesis-generator",
            "model-architect",
            "analysis-planner",
            "sampling-strategist",
            "instrument-developer",
            "validity-guardian",
            "methodology-scanner",
            "methodology-writer",
        ],
    },
    ResearchPhase {
        id: 6,
        name: "Writing",
        description: "Document creation including introduction, literature review, results, discussion, conclusion, and abstract chapters.",
        agent_keys: &[
            "introduction-writer",
            "literature-review-writer",
            "results-writer",
            "discussion-writer",
            "conclusion-writer",
            "abstract-writer",
        ],
    },
    ResearchPhase {
        id: 7,
        name: "Validation",
        description: "Final quality assurance including systematic review, ethics review, adversarial review, confidence quantification, citation validation, reproducibility checking, APA formatting, consistency validation, quality assessment, bias detection, and file length management.",
        agent_keys: &[
            "systematic-reviewer",
            "ethics-reviewer",
            "adversarial-reviewer",
            "confidence-quantifier",
            "citation-validator",
            "reproducibility-checker",
            "apa-citation-specialist",
            "consistency-validator",
            "quality-assessor",
            "bias-detector",
            "file-length-manager",
        ],
    },
];

// ---------------------------------------------------------------------------
// Helper functions
// ---------------------------------------------------------------------------

/// Returns a reference to all 46 research agents.
pub fn get_all_agents() -> &'static [ResearchAgent] {
    RESEARCH_AGENTS
}

/// Returns agents belonging to the given phase number (1-7).
pub fn get_agents_by_phase(phase: u8) -> Vec<&'static ResearchAgent> {
    RESEARCH_AGENTS
        .iter()
        .filter(|a| a.phase == phase)
        .collect()
}

/// Looks up a research agent by its unique key.
pub fn get_agent_by_key(key: &str) -> Option<&'static ResearchAgent> {
    RESEARCH_AGENTS.iter().find(|a| a.key == key)
}

/// Returns the 0-based index of the agent with the given key.
pub fn get_agent_index(key: &str) -> Option<usize> {
    RESEARCH_AGENTS.iter().position(|a| a.key == key)
}

/// Looks up a research phase by its ID (1-7).
pub fn get_phase_by_id(id: u8) -> Option<&'static ResearchPhase> {
    RESEARCH_PHASES.iter().find(|p| p.id == id)
}

/// Validates that phase agent_keys match agent definitions and counts are consistent.
pub fn validate_configuration() -> Result<(), String> {
    let agent_keys: std::collections::HashSet<&str> =
        RESEARCH_AGENTS.iter().map(|a| a.key).collect();

    for phase in RESEARCH_PHASES.iter() {
        for agent_key in phase.agent_keys.iter() {
            if !agent_keys.contains(agent_key) {
                return Err(format!(
                    "Phase {} ({}) references unknown agent \"{}\"",
                    phase.id, phase.name, agent_key
                ));
            }
        }
    }

    let phase_agent_count: usize = RESEARCH_PHASES.iter().map(|p| p.agent_keys.len()).sum();
    if phase_agent_count != RESEARCH_AGENTS.len() {
        return Err(format!(
            "Phase agent count ({}) does not match total agents ({})",
            phase_agent_count,
            RESEARCH_AGENTS.len()
        ));
    }

    // Verify each agent's phase matches the phase that lists it
    for phase in RESEARCH_PHASES.iter() {
        for agent_key in phase.agent_keys.iter() {
            if let Some(agent) = get_agent_by_key(agent_key) {
                if agent.phase != phase.id {
                    return Err(format!(
                        "Agent \"{}\" has phase {} but is listed in phase {}",
                        agent_key, agent.phase, phase.id
                    ));
                }
            }
        }
    }

    Ok(())
}
