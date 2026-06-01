//! Semantic citation gates for the research pipeline.

const FINAL_PAPER_MIN_WORDS: usize = 6_000;

/// Returns a hard-gate failure reason for citation/final outputs.
pub fn hard_failure(agent_key: &str, output: &str) -> Option<String> {
    match agent_key {
        "citation-reconciler" => reconciler_failure(output),
        "file-length-manager" => file_length_failure(output),
        "chapter-synthesizer" => final_paper_failure(output),
        _ => None,
    }
}

fn reconciler_failure(output: &str) -> Option<String> {
    let lower = normalized(output);
    if contains_failure_marker(&lower) {
        return Some("citation reconciliation still reports unresolved failures".into());
    }
    if !lower.contains("citation repair status") || !lower.contains("pass") {
        return Some("citation reconciler must emit `Citation Repair Status: PASS`".into());
    }
    if !lower.contains("master reference list") {
        return Some("citation reconciler must emit a master reference list".into());
    }
    None
}

fn file_length_failure(output: &str) -> Option<String> {
    let lower = normalized(output);
    if status_is_fail(&lower, "length cap status")
        || status_is_fail(&lower, "chapter source coverage")
        || status_is_fail(&lower, "final assembly readiness")
    {
        return Some("file length manager reports final assembly is not ready".into());
    }
    if !status_is_pass(&lower, "length cap status") {
        return Some("file length manager must emit `Length Cap Status: PASS`".into());
    }
    if !status_is_pass(&lower, "chapter source coverage") {
        return Some("file length manager must emit `Chapter Source Coverage: PASS`".into());
    }
    if !status_is_pass(&lower, "final assembly readiness") {
        return Some("file length manager must emit `Final Assembly Readiness: PASS`".into());
    }
    None
}

fn final_paper_failure(output: &str) -> Option<String> {
    let lower = normalized(output);
    if contains_failure_marker(&lower) {
        return Some("final paper still contains unresolved citation failure language".into());
    }
    if word_count(output) < FINAL_PAPER_MIN_WORDS {
        return Some(format!(
            "final paper must be at least {FINAL_PAPER_MIN_WORDS} words"
        ));
    }
    if !lower.contains("## abstract") {
        return Some("final paper must include a `## Abstract` section".into());
    }
    if !lower.contains("## introduction") && !lower.contains("## 1. introduction") {
        return Some("final paper must include an Introduction section".into());
    }
    if !lower.contains("## references") {
        return Some("final paper must include exactly one `## References` section".into());
    }
    if lower.matches("## references").count() != 1 {
        return Some("final paper must not contain duplicate References sections".into());
    }
    None
}

fn contains_failure_marker(lower: &str) -> bool {
    const MARKERS: &[&str] = &[
        "needs revision before publication",
        "not currently publication-ready",
        "publication readiness | not ready",
        "citation repair status: fail",
        "citation repair status: \u{274c}",
        "citation integrity status: fail",
        "citation integrity status: \u{274c}",
        "reference list completeness | fail",
        "cross-reference integrity | fail",
        "every in-text citation has reference entry | \u{274c}",
        "invalid cross-reference integrity",
        "unresolved orphaned in-text citations",
        "remaining orphaned in-text citations",
    ];
    MARKERS.iter().any(|marker| lower.contains(marker))
        || (lower.contains("unresolved") && lower.contains("author/year mismatch"))
}

fn normalized(output: &str) -> String {
    output.replace("**", "").replace('`', "").to_lowercase()
}

fn status_is_pass(lower: &str, label: &str) -> bool {
    lower
        .lines()
        .any(|line| line.contains(label) && line.contains("pass"))
}

fn status_is_fail(lower: &str, label: &str) -> bool {
    lower
        .lines()
        .any(|line| line.contains(label) && line.contains("fail"))
}

fn word_count(output: &str) -> usize {
    output
        .split_whitespace()
        .filter(|word| word.chars().any(char::is_alphanumeric))
        .count()
}

#[cfg(test)]
mod tests {
    use super::hard_failure;

    #[test]
    fn reconciler_rejects_revision_report() {
        let output = "Status: NEEDS REVISION BEFORE PUBLICATION\n\
            Every in-text citation has reference entry | \u{274c}";
        assert!(hard_failure("citation-reconciler", output).is_some());
    }

    #[test]
    fn reconciler_accepts_pass_with_master_references() {
        let output = "# Citation Repair\n\
            **Citation Repair Status**: PASS\n\
            ## Master Reference List\n\
            Boris FX. (2026). *Sequoia user manual*.";
        assert!(hard_failure("citation-reconciler", output).is_none());
    }

    #[test]
    fn reconciler_can_note_removed_topic_specific_citation() {
        let output = "# Citation Repair\n\
            **Citation Repair Status**: PASS\n\
            ## Master Reference List\n\
            Boris FX. (2026). *Sequoia user manual*.\n\
            ## Removed or Downgraded Citations\n\
            An unsupported forum citation was removed from the final reference list.";
        assert!(hard_failure("citation-reconciler", output).is_none());
    }

    #[test]
    fn final_paper_rejects_unresolved_gate_language() {
        let output = "# Paper\n\n## References\n\nCitation integrity status: FAIL";
        assert!(hard_failure("chapter-synthesizer", output).is_some());
    }

    #[test]
    fn file_length_manager_rejects_line_cap_only_report() {
        let output = "# File Length Management Report\n\
            Status: PASS - No Markdown artifact exceeds 1,500 lines";
        assert!(hard_failure("file-length-manager", output).is_some());
    }

    #[test]
    fn file_length_manager_rejects_failed_source_coverage() {
        let output = "Length Cap Status: PASS\n\
            Chapter Source Coverage: FAIL\n\
            Final Assembly Readiness: FAIL";
        assert!(hard_failure("file-length-manager", output).is_some());
    }

    #[test]
    fn file_length_manager_accepts_complete_readiness_status() {
        let output = "Length Cap Status: PASS\n\
            Chapter Source Coverage: PASS\n\
            Final Assembly Readiness: PASS";
        assert!(hard_failure("file-length-manager", output).is_none());
    }

    #[test]
    fn final_paper_rejects_short_output() {
        let output =
            "# Paper\n\n## Abstract\n\nBrief.\n\n## Introduction\n\nBrief.\n\n## References";
        let failure = hard_failure("chapter-synthesizer", output).unwrap();
        assert!(failure.contains("at least 6000 words"));
    }
}
