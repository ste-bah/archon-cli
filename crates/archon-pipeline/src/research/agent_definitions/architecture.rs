use super::super::{BASE_TOOLS, ResearchAgent};

pub(in super::super) const THEORETICAL_FRAMEWORK_ANALYST: ResearchAgent = ResearchAgent {
    key: "theoretical-framework-analyst",
    display_name: "Theoretical Framework Analyst",
    phase: 3,
    file: "theoretical-framework-analyst.md",
    memory_keys: &["research/foundation/framework", "research/theory/analysis"],
    output_artifacts: &["theoretical-framework.md", "framework-map.md"],
    prompt_source_path: ".archon/agents/phdresearch/theoretical-framework-analyst.md",
    tool_access: BASE_TOOLS,
};

pub(in super::super) const CONTRADICTION_ANALYZER: ResearchAgent = ResearchAgent {
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
};

pub(in super::super) const GAP_HUNTER: ResearchAgent = ResearchAgent {
    key: "gap-hunter",
    display_name: "Gap Hunter",
    phase: 3,
    file: "gap-hunter.md",
    memory_keys: &["research/analysis/gaps", "research/findings/gaps"],
    output_artifacts: &["research-gaps.md", "gap-priorities.md"],
    prompt_source_path: ".archon/agents/phdresearch/gap-hunter.md",
    tool_access: BASE_TOOLS,
};

pub(in super::super) const RISK_ANALYST: ResearchAgent = ResearchAgent {
    key: "risk-analyst",
    display_name: "Risk Analyst",
    phase: 3,
    file: "risk-analyst.md",
    memory_keys: &["research/analysis/risks", "research/meta/risks"],
    output_artifacts: &["risk-assessment.md", "risk-mitigation.md"],
    prompt_source_path: ".archon/agents/phdresearch/risk-analyst.md",
    tool_access: BASE_TOOLS,
};
