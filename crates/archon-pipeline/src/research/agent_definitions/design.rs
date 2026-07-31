use super::super::{BASE_TOOLS, ResearchAgent};

pub(in super::super) const METHOD_DESIGNER: ResearchAgent = ResearchAgent {
    key: "method-designer",
    display_name: "Method Designer",
    phase: 5,
    file: "method-designer.md",
    memory_keys: &["research/methods/design", "research/methodology/approach"],
    output_artifacts: &["research-design.md", "method-rationale.md"],
    prompt_source_path: ".archon/agents/phdresearch/method-designer.md",
    tool_access: BASE_TOOLS,
};

pub(in super::super) const HYPOTHESIS_GENERATOR: ResearchAgent = ResearchAgent {
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
};

pub(in super::super) const MODEL_ARCHITECT: ResearchAgent = ResearchAgent {
    key: "model-architect",
    display_name: "Model Architect",
    phase: 5,
    file: "model-architect.md",
    memory_keys: &["research/synthesis/models", "research/theory/models"],
    output_artifacts: &["conceptual-model.md", "model-specifications.md"],
    prompt_source_path: ".archon/agents/phdresearch/model-architect.md",
    tool_access: BASE_TOOLS,
};

pub(in super::super) const ANALYSIS_PLANNER: ResearchAgent = ResearchAgent {
    key: "analysis-planner",
    display_name: "Analysis Planner",
    phase: 5,
    file: "analysis-planner.md",
    memory_keys: &["research/methods/analysis", "research/methodology/analysis"],
    output_artifacts: &["analysis-plan.md", "statistical-approach.md"],
    prompt_source_path: ".archon/agents/phdresearch/analysis-planner.md",
    tool_access: BASE_TOOLS,
};

pub(in super::super) const SAMPLING_STRATEGIST: ResearchAgent = ResearchAgent {
    key: "sampling-strategist",
    display_name: "Sampling Strategist",
    phase: 5,
    file: "sampling-strategist.md",
    memory_keys: &["research/methods/sampling", "research/methodology/sampling"],
    output_artifacts: &["sampling-strategy.md", "sample-specifications.md"],
    prompt_source_path: ".archon/agents/phdresearch/sampling-strategist.md",
    tool_access: BASE_TOOLS,
};

pub(in super::super) const INSTRUMENT_DEVELOPER: ResearchAgent = ResearchAgent {
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
};

pub(in super::super) const VALIDITY_GUARDIAN: ResearchAgent = ResearchAgent {
    key: "validity-guardian",
    display_name: "Validity Guardian",
    phase: 5,
    file: "validity-guardian.md",
    memory_keys: &["research/methods/validity", "research/quality/validity"],
    output_artifacts: &["validity-assessment.md", "threat-mitigation.md"],
    prompt_source_path: ".archon/agents/phdresearch/validity-guardian.md",
    tool_access: BASE_TOOLS,
};

pub(in super::super) const METHODOLOGY_SCANNER: ResearchAgent = ResearchAgent {
    key: "methodology-scanner",
    display_name: "Methodology Scanner",
    phase: 5,
    file: "methodology-scanner.md",
    memory_keys: &["research/literature/methods", "research/methodology/survey"],
    output_artifacts: &["methodology-survey.md", "method-comparison.md"],
    prompt_source_path: ".archon/agents/phdresearch/methodology-scanner.md",
    tool_access: BASE_TOOLS,
};

pub(in super::super) const METHODOLOGY_WRITER: ResearchAgent = ResearchAgent {
    key: "methodology-writer",
    display_name: "Methodology Writer",
    phase: 5,
    file: "methodology-writer.md",
    memory_keys: &["research/writing/methodology", "research/document/chapter3"],
    output_artifacts: &["methodology-chapter.md", "method-details.md"],
    prompt_source_path: ".archon/agents/phdresearch/methodology-writer.md",
    tool_access: BASE_TOOLS,
};
