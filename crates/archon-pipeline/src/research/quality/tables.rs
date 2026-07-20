use std::collections::HashMap;
use std::sync::LazyLock;

/// Minimum expected word counts per agent key.
pub(super) static AGENT_MIN_LENGTHS: LazyLock<HashMap<&'static str, usize>> = LazyLock::new(|| {
    HashMap::from([
        ("literature-review-writer", 8000),
        ("introduction-writer", 5000),
        ("discussion-writer", 5000),
        ("methodology-writer", 4000),
        ("results-writer", 4000),
        ("conclusion-writer", 3000),
        ("abstract-writer", 300),
        ("citation-reconciler", 1500),
        ("chapter-synthesizer", 6000),
        ("systematic-reviewer", 5000),
        ("literature-mapper", 3000),
        ("evidence-synthesizer", 3000),
        ("thematic-synthesizer", 2500),
        ("theory-builder", 2500),
        ("method-designer", 2000),
        ("hypothesis-generator", 1500),
        ("model-architect", 2000),
        ("instrument-developer", 2000),
        ("sampling-strategist", 1500),
        ("analysis-planner", 1500),
        ("step-back-analyzer", 1500),
        ("contradiction-analyzer", 2000),
        ("gap-hunter", 1500),
        ("self-ask-decomposer", 1000),
    ])
});

pub(super) static CRITICAL_AGENTS: &[&str] = &[
    "step-back-analyzer",
    "contradiction-analyzer",
    "gap-hunter",
    "theoretical-framework-analyst",
    "bias-detector",
    "quality-assessor",
    "validity-guardian",
    "introduction-writer",
    "literature-review-writer",
    "methodology-writer",
    "results-writer",
    "discussion-writer",
    "conclusion-writer",
    "chapter-synthesizer",
    "abstract-writer",
    "citation-reconciler",
    "file-length-manager",
];

pub(super) static WRITING_AGENTS: &[&str] = &[
    "introduction-writer",
    "literature-review-writer",
    "methodology-writer",
    "results-writer",
    "discussion-writer",
    "conclusion-writer",
    "chapter-synthesizer",
    "abstract-writer",
];

/// Expected sections per writing agent (keywords searched case-insensitively).
pub(super) static AGENT_EXPECTED_SECTIONS: LazyLock<HashMap<&'static str, Vec<&'static str>>> =
    LazyLock::new(|| {
        HashMap::from([
            (
                "introduction-writer",
                vec![
                    "background",
                    "problem statement",
                    "research questions",
                    "significance",
                    "scope",
                ],
            ),
            (
                "literature-review-writer",
                vec![
                    "theoretical framework",
                    "key themes",
                    "gaps",
                    "synthesis",
                    "summary",
                ],
            ),
            (
                "methodology-writer",
                vec![
                    "research design",
                    "data collection",
                    "sampling",
                    "analysis",
                    "validity",
                    "ethics",
                ],
            ),
            (
                "results-writer",
                vec!["findings", "analysis", "themes", "patterns", "summary"],
            ),
            (
                "discussion-writer",
                vec![
                    "interpretation",
                    "implications",
                    "limitations",
                    "comparison",
                    "recommendations",
                ],
            ),
            (
                "conclusion-writer",
                vec![
                    "summary",
                    "contributions",
                    "limitations",
                    "future research",
                    "final remarks",
                ],
            ),
            (
                "abstract-writer",
                vec!["purpose", "method", "results", "conclusions"],
            ),
        ])
    });

pub(super) static ACADEMIC_MARKERS: &[&str] = &[
    "methodology",
    "framework",
    "hypothesis",
    "empirical",
    "theoretical",
    "systematic",
    "analysis",
    "findings",
    "implications",
    "limitations",
    "literature",
    "qualitative",
    "quantitative",
    "validity",
    "reliability",
];

pub(super) static METHODOLOGY_PATTERNS: &[&str] = &[
    "research design",
    "data collection",
    "sampling",
    "interview",
    "survey",
    "case study",
    "ethnography",
    "grounded theory",
    "phenomenology",
    "content analysis",
    "meta-analysis",
];

pub(super) static STATISTICAL_PATTERNS: &[&str] = &[
    "p-value",
    "correlation",
    "regression",
    "significant",
    "standard deviation",
    "mean",
    "median",
    "chi-square",
];

pub(super) static EVIDENCE_LANGUAGE: &[&str] = &[
    "evidence suggests",
    "findings indicate",
    "results show",
    "data reveals",
    "analysis demonstrates",
    "research confirms",
];
