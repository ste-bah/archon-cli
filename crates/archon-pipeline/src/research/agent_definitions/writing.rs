use super::super::{ResearchAgent, WRITER_TOOLS};

pub(in super::super) const INTRODUCTION_WRITER: ResearchAgent = ResearchAgent {
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
};

pub(in super::super) const LITERATURE_REVIEW_WRITER: ResearchAgent = ResearchAgent {
    key: "literature-review-writer",
    display_name: "Literature Review Writer",
    phase: 6,
    file: "literature-review-writer.md",
    memory_keys: &["research/writing/literature", "research/document/chapter2"],
    output_artifacts: &["literature-review.md", "synthesis-narrative.md"],
    prompt_source_path: ".archon/agents/phdresearch/literature-review-writer.md",
    tool_access: WRITER_TOOLS,
};

pub(in super::super) const RESULTS_WRITER: ResearchAgent = ResearchAgent {
    key: "results-writer",
    display_name: "Results Writer",
    phase: 6,
    file: "results-writer.md",
    memory_keys: &["research/writing/results", "research/document/chapter4"],
    output_artifacts: &["results-chapter.md", "findings-narrative.md"],
    prompt_source_path: ".archon/agents/phdresearch/results-writer.md",
    tool_access: WRITER_TOOLS,
};

pub(in super::super) const DISCUSSION_WRITER: ResearchAgent = ResearchAgent {
    key: "discussion-writer",
    display_name: "Discussion Writer",
    phase: 6,
    file: "discussion-writer.md",
    memory_keys: &["research/writing/discussion", "research/document/chapter5"],
    output_artifacts: &["discussion-chapter.md", "implications.md"],
    prompt_source_path: ".archon/agents/phdresearch/discussion-writer.md",
    tool_access: WRITER_TOOLS,
};

pub(in super::super) const CONCLUSION_WRITER: ResearchAgent = ResearchAgent {
    key: "conclusion-writer",
    display_name: "Conclusion Writer",
    phase: 6,
    file: "conclusion-writer.md",
    memory_keys: &["research/writing/conclusion", "research/document/chapter6"],
    output_artifacts: &["conclusion-chapter.md", "future-directions.md"],
    prompt_source_path: ".archon/agents/phdresearch/conclusion-writer.md",
    tool_access: WRITER_TOOLS,
};

pub(in super::super) const ABSTRACT_WRITER: ResearchAgent = ResearchAgent {
    key: "abstract-writer",
    display_name: "Abstract Writer",
    phase: 6,
    file: "abstract-writer.md",
    memory_keys: &["research/writing/abstract", "research/document/abstract"],
    output_artifacts: &["abstract.md", "executive-summary.md"],
    prompt_source_path: ".archon/agents/phdresearch/abstract-writer.md",
    tool_access: WRITER_TOOLS,
};
