use super::super::{BASE_TOOLS, ResearchAgent};

pub(in super::super) const SYSTEMATIC_REVIEWER: ResearchAgent = ResearchAgent {
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
};

pub(in super::super) const ETHICS_REVIEWER: ResearchAgent = ResearchAgent {
    key: "ethics-reviewer",
    display_name: "Ethics Reviewer",
    phase: 7,
    file: "ethics-reviewer.md",
    memory_keys: &["research/methods/ethics", "research/compliance/ethics"],
    output_artifacts: &["ethics-review.md", "irb-protocol.md"],
    prompt_source_path: ".archon/agents/phdresearch/ethics-reviewer.md",
    tool_access: BASE_TOOLS,
};

pub(in super::super) const ADVERSARIAL_REVIEWER: ResearchAgent = ResearchAgent {
    key: "adversarial-reviewer",
    display_name: "Adversarial Reviewer",
    phase: 7,
    file: "adversarial-reviewer.md",
    memory_keys: &["research/quality/critique", "research/review/adversarial"],
    output_artifacts: &["adversarial-critique.md", "weakness-report.md"],
    prompt_source_path: ".archon/agents/phdresearch/adversarial-reviewer.md",
    tool_access: BASE_TOOLS,
};

pub(in super::super) const CONFIDENCE_QUANTIFIER: ResearchAgent = ResearchAgent {
    key: "confidence-quantifier",
    display_name: "Confidence Quantifier",
    phase: 7,
    file: "confidence-quantifier.md",
    memory_keys: &["research/quality/confidence", "research/meta/certainty"],
    output_artifacts: &["confidence-scores.md", "uncertainty-analysis.md"],
    prompt_source_path: ".archon/agents/phdresearch/confidence-quantifier.md",
    tool_access: BASE_TOOLS,
};

pub(in super::super) const CITATION_VALIDATOR: ResearchAgent = ResearchAgent {
    key: "citation-validator",
    display_name: "Citation Validator",
    phase: 7,
    file: "citation-validator.md",
    memory_keys: &["research/quality/validation", "research/sources/verified"],
    output_artifacts: &["citation-validation.md", "source-verification.md"],
    prompt_source_path: ".archon/agents/phdresearch/citation-validator.md",
    tool_access: BASE_TOOLS,
};

pub(in super::super) const REPRODUCIBILITY_CHECKER: ResearchAgent = ResearchAgent {
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
};

pub(in super::super) const APA_CITATION_SPECIALIST: ResearchAgent = ResearchAgent {
    key: "apa-citation-specialist",
    display_name: "APA Citation Specialist",
    phase: 7,
    file: "apa-citation-specialist.md",
    memory_keys: &["research/quality/citations", "research/document/references"],
    output_artifacts: &["citation-audit.md", "apa-compliance.md"],
    prompt_source_path: ".archon/agents/phdresearch/apa-citation-specialist.md",
    tool_access: BASE_TOOLS,
};

pub(in super::super) const CITATION_RECONCILER: ResearchAgent = ResearchAgent {
    key: "citation-reconciler",
    display_name: "Citation Reconciler",
    phase: 7,
    file: "citation-reconciler.md",
    memory_keys: &[
        "research/quality/citations",
        "research/document/references",
        "research/quality/citation-repair",
        "research/sources/verified",
    ],
    output_artifacts: &["citation-reconciliation.md", "master-reference-list.md"],
    prompt_source_path: ".archon/agents/phdresearch/citation-reconciler.md",
    tool_access: BASE_TOOLS,
};

pub(in super::super) const CONSISTENCY_VALIDATOR: ResearchAgent = ResearchAgent {
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
};

pub(in super::super) const QUALITY_ASSESSOR: ResearchAgent = ResearchAgent {
    key: "quality-assessor",
    display_name: "Quality Assessor",
    phase: 7,
    file: "quality-assessor.md",
    memory_keys: &["research/analysis/quality", "research/meta/assessment"],
    output_artifacts: &["quality-assessment.md", "quality-scores.md"],
    prompt_source_path: ".archon/agents/phdresearch/quality-assessor.md",
    tool_access: BASE_TOOLS,
};

pub(in super::super) const BIAS_DETECTOR: ResearchAgent = ResearchAgent {
    key: "bias-detector",
    display_name: "Bias Detector",
    phase: 7,
    file: "bias-detector.md",
    memory_keys: &["research/analysis/bias", "research/quality/bias"],
    output_artifacts: &["bias-analysis.md", "bias-mitigation.md"],
    prompt_source_path: ".archon/agents/phdresearch/bias-detector.md",
    tool_access: BASE_TOOLS,
};

pub(in super::super) const FILE_LENGTH_MANAGER: ResearchAgent = ResearchAgent {
    key: "file-length-manager",
    display_name: "File Length Manager",
    phase: 7,
    file: "file-length-manager.md",
    memory_keys: &["research/quality/structure", "research/document/formatting"],
    output_artifacts: &["structure-audit.md", "length-compliance.md"],
    prompt_source_path: ".archon/agents/phdresearch/file-length-manager.md",
    tool_access: BASE_TOOLS,
};
