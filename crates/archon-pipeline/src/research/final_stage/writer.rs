//! Chapter writer — assembles a chapter body from its mapped agent outputs.
//!
//! This is the seam for the phase-8 `chapter-synthesizer` agent
//! (`.archon/agents/phdresearch/pipeline.toml`), the single agent of the
//! "Final Assembly" phase. The phase-6 writers — `introduction-writer`,
//! `literature-review-writer`, `results-writer`, `discussion-writer`,
//! `conclusion-writer`, `abstract-writer` — produce the sources it consumes.
//!
//! Until then it concatenates. That concatenation used to be inlined in
//! `FinalStageOrchestrator::run`, so this module had no caller at all and the
//! seam did not exist in the running code — in a library crate `pub` keeps
//! `dead_code` quiet, so nothing said so. The logic now lives here, which is
//! where synthesis replaces it.
//!
//! It deliberately emits **no heading**. `combiner::combine_chapters` writes
//! `## Chapter {n}: {title}` for every chapter; the earlier version of this
//! function opened with `## {title}`, so wiring it in as written would have
//! double-headed the whole paper.
//!
//! TODO(REQ-RESEARCH-007): Replace the concatenation with LLM-driven chapter
//! synthesis that uses `LlmClient` for coherent academic prose, driven by the
//! `chapter-synthesizer` agent definition.

/// Build a chapter body from the agent outputs mapped to it.
///
/// Sources are joined in the order the mapper produced them, blank-line
/// separated. `title` is not part of the output — the combiner owns headings —
/// but it is what the synthesis prompt will be written around, so it is part
/// of the signature now rather than after the fact.
///
/// An empty `sources` slice yields an empty body. Callers decide what an
/// unmapped chapter should say; `FinalStageOrchestrator` substitutes
/// `generate_chapter_placeholder` rather than emitting a blank chapter.
pub fn synthesize_chapter(title: &str, sources: &[&str]) -> String {
    tracing::debug!(
        chapter = title,
        sources = sources.len(),
        "assembling chapter body from mapped agent outputs"
    );
    sources.join("\n\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn joins_sources_with_a_blank_line() {
        let body = synthesize_chapter("Introduction", &["first para", "second para"]);
        assert_eq!(body, "first para\n\nsecond para");
    }

    #[test]
    fn emits_no_heading_because_the_combiner_owns_them() {
        let body = synthesize_chapter("Methodology", &["some prose"]);
        assert!(
            !body.starts_with('#'),
            "a heading here double-heads every chapter: {body:?}"
        );
        assert!(
            !body.contains("Methodology"),
            "the title must not leak into the body: {body:?}"
        );
    }

    #[test]
    fn no_sources_yields_an_empty_body_for_the_caller_to_replace() {
        assert_eq!(synthesize_chapter("Results", &[]), "");
    }
}
