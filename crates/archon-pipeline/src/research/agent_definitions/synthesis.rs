use super::super::{ResearchAgent, BASE_TOOLS, WRITER_TOOLS};

pub(super) const EVIDENCE_SYNTHESIZER: ResearchAgent = ResearchAgent {
        key: "evidence-synthesizer",
        display_name: "Evidence Synthesizer",
        phase: 4,
        file: "evidence-synthesizer.md",
        memory_keys: &["research/analysis/evidence", "research/synthesis/evidence"],
        output_artifacts: &["evidence-synthesis.md", "evidence-matrix.md"],
        prompt_source_path: ".archon/agents/phdresearch/evidence-synthesizer.md",
        tool_access: BASE_TOOLS,
    };

pub(super) const PATTERN_ANALYST: ResearchAgent = ResearchAgent {
        key: "pattern-analyst",
        display_name: "Pattern Analyst",
        phase: 4,
        file: "pattern-analyst.md",
        memory_keys: &["research/synthesis/patterns", "research/findings/patterns"],
        output_artifacts: &["pattern-analysis.md", "pattern-catalog.md"],
        prompt_source_path: ".archon/agents/phdresearch/pattern-analyst.md",
        tool_access: BASE_TOOLS,
    };

pub(super) const THEMATIC_SYNTHESIZER: ResearchAgent = ResearchAgent {
        key: "thematic-synthesizer",
        display_name: "Thematic Synthesizer",
        phase: 4,
        file: "thematic-synthesizer.md",
        memory_keys: &["research/synthesis/themes", "research/findings/themes"],
        output_artifacts: &["thematic-synthesis.md", "theme-hierarchy.md"],
        prompt_source_path: ".archon/agents/phdresearch/thematic-synthesizer.md",
        tool_access: BASE_TOOLS,
    };

pub(super) const THEORY_BUILDER: ResearchAgent = ResearchAgent {
        key: "theory-builder",
        display_name: "Theory Builder",
        phase: 4,
        file: "theory-builder.md",
        memory_keys: &["research/synthesis/theory", "research/theory/construction"],
        output_artifacts: &["theory-development.md", "theoretical-model.md"],
        prompt_source_path: ".archon/agents/phdresearch/theory-builder.md",
        tool_access: BASE_TOOLS,
    };

pub(super) const OPPORTUNITY_IDENTIFIER: ResearchAgent = ResearchAgent {
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
    };
