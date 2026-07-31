use super::super::{ResearchAgent, BASE_TOOLS, WRITER_TOOLS};

pub(super) const STEP_BACK_ANALYZER: ResearchAgent = ResearchAgent {
        key: "step-back-analyzer",
        display_name: "Step-Back Analyzer",
        phase: 1,
        file: "step-back-analyzer.md",
        memory_keys: &["research/foundation/framing", "research/meta/perspective"],
        output_artifacts: &["high-level-framing.md", "abstraction-analysis.md"],
        prompt_source_path: ".archon/agents/phdresearch/step-back-analyzer.md",
        tool_access: BASE_TOOLS,
    };

pub(super) const SELF_ASK_DECOMPOSER: ResearchAgent = ResearchAgent {
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
    };

pub(super) const AMBIGUITY_CLARIFIER: ResearchAgent = ResearchAgent {
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
    };

pub(super) const RESEARCH_PLANNER: ResearchAgent = ResearchAgent {
        key: "research-planner",
        display_name: "Research Planner",
        phase: 1,
        file: "research-planner.md",
        memory_keys: &["research/foundation/plan", "research/meta/strategy"],
        output_artifacts: &["research-plan.md", "timeline.md"],
        prompt_source_path: ".archon/agents/phdresearch/research-planner.md",
        tool_access: BASE_TOOLS,
    };

pub(super) const CONSTRUCT_DEFINER: ResearchAgent = ResearchAgent {
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
    };

pub(super) const DISSERTATION_ARCHITECT: ResearchAgent = ResearchAgent {
        key: "dissertation-architect",
        display_name: "Dissertation Architect",
        phase: 1,
        file: "dissertation-architect.md",
        memory_keys: &[
            "research/structure/chapters",
            "research/writing/structure",
            "research/document/architecture",
        ],
        output_artifacts: &["dissertation-outline.md", "chapter-structure.md"],
        prompt_source_path: ".archon/agents/phdresearch/dissertation-architect.md",
        tool_access: BASE_TOOLS,
    };
