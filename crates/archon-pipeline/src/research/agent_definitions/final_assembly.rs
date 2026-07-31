use super::super::{ResearchAgent, WRITER_TOOLS};

pub(in super::super) const CHAPTER_SYNTHESIZER: ResearchAgent = ResearchAgent {
    key: "chapter-synthesizer",
    display_name: "Chapter Synthesizer",
    phase: 8,
    file: "chapter-synthesizer.md",
    memory_keys: &[
        "research/document/final",
        "research/structure/chapters",
        "research/writing/introduction",
        "research/writing/literature",
        "research/writing/methodology",
        "research/writing/results",
        "research/writing/discussion",
        "research/writing/conclusion",
        "research/writing/abstract",
        "research/quality/citations",
        "research/quality/validation",
        "research/quality/citation-repair",
        "research/document/references",
        "research/quality/consistency",
        "research/quality/structure",
    ],
    output_artifacts: &["final-paper.md", "dissertation-complete.md"],
    prompt_source_path: ".archon/agents/phdresearch/chapter-synthesizer.md",
    tool_access: WRITER_TOOLS,
};
