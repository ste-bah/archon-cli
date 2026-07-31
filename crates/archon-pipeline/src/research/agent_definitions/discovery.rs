use super::super::{BASE_TOOLS, ResearchAgent};

pub(in super::super) const LITERATURE_MAPPER: ResearchAgent = ResearchAgent {
    key: "literature-mapper",
    display_name: "Literature Mapper",
    phase: 2,
    file: "literature-mapper.md",
    memory_keys: &["research/literature/map", "research/sources/index"],
    output_artifacts: &["literature-map.md", "source-catalog.md"],
    prompt_source_path: ".archon/agents/phdresearch/literature-mapper.md",
    tool_access: BASE_TOOLS,
};

pub(in super::super) const SOURCE_TIER_CLASSIFIER: ResearchAgent = ResearchAgent {
    key: "source-tier-classifier",
    display_name: "Source Tier Classifier",
    phase: 2,
    file: "source-tier-classifier.md",
    memory_keys: &["research/literature/tiers", "research/quality/sources"],
    output_artifacts: &["source-tiers.md", "credibility-assessment.md"],
    prompt_source_path: ".archon/agents/phdresearch/source-tier-classifier.md",
    tool_access: BASE_TOOLS,
};

pub(in super::super) const CITATION_EXTRACTOR: ResearchAgent = ResearchAgent {
    key: "citation-extractor",
    display_name: "Citation Extractor",
    phase: 2,
    file: "citation-extractor.md",
    memory_keys: &["research/quality/extraction", "research/sources/citations"],
    output_artifacts: &["extracted-citations.md", "reference-list.md"],
    prompt_source_path: ".archon/agents/phdresearch/citation-extractor.md",
    tool_access: BASE_TOOLS,
};

pub(in super::super) const CONTEXT_TIER_MANAGER: ResearchAgent = ResearchAgent {
    key: "context-tier-manager",
    display_name: "Context Tier Manager",
    phase: 2,
    file: "context-tier-manager.md",
    memory_keys: &["research/literature/context", "research/meta/tiers"],
    output_artifacts: &["context-hierarchy.md", "tier-mappings.md"],
    prompt_source_path: ".archon/agents/phdresearch/context-tier-manager.md",
    tool_access: BASE_TOOLS,
};
