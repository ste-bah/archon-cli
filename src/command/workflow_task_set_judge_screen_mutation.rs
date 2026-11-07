//! Only a recognised host probe and its closed set of consequences are screened.
use super::*;

pub(super) const MUTATION_FACT: &str = "The host may move named files aside to prove a check can fail. That is a host probe, not an implementation state, so it is never a counterexample. Preserve any implementation-state objection. Judge the deciding test as supplied, or give an independent behavioural mechanism that passes while the criterion is false.";

pub(super) fn mutation_only(decision: &JudgeResponse) -> bool {
    if decision.verdict != JudgeDecision::Refuted {
        return false;
    }
    static ACTION: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(
            r#"(?ix)^\s*the\s+(?:host(?:['’]s\s+(?:mutation\s+)?probe|\s+mutation\s+probe)?|mutation\s+probe)\s+
            (?:(?:moves?|puts?)\s+(?P<moved>`[^`]+`|"[^"]+"|'[^']+'|[^\s,;]+)\s+aside\b
            |renames?\s+(?P<renamed>`[^`]+`|"[^"]+"|'[^']+'|[^\s,;]+?)(?:\s+to\s+(?:`[^`]+`|"[^"]+"|'[^']+'|[^\s,;]+?))?(?:[,;.!](?:\s|$)|$))"#,
        )
        .expect("host file action expression")
    });
    static CLAUSES: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        // A period inside a path is not a sentence end.
        regex::Regex::new(r"(?ix)[.!?](?:\s+|$)|[,;]|\b(?:so|and|then|while)\b")
            .expect("probe clause boundaries")
    });
    static CONSEQUENCE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        // Full clauses only: any extra implementation/state wording forwards.
        // These are probe outcomes, never an open-ended objection vocabulary.
        regex::Regex::new(
            r"(?ix)^(?:
                (?:the\s+)?check\s+passes(?:\s+trivially)?
                |(?:cargo\s+test(?:\s+(?:--\s+)?--exact)?|--exact)\s+(?:passes(?:\s+trivially)?|exits\s+0|succeeds)(?:\s+with\s+no\s+matching\s+tests)?
                |(?:zero|no)\s+tests\s+(?:run|match)
                |--exact\s+matches\s+zero\s+tests
                |no\s+deciding\s+tests?\s+runs?
                |no\s+behaviou?ral\s+proof
                |no\s+assertions\s+remain
                |(?:the\s+criterion|[a-z][a-z0-9]*(?:-[a-z0-9]+)+)\s+has\s+no\s+deciding\s+test
                |matching\s+zero\s+tests\s+is\s+success\s+for\s+cargo\s+test
                |moving\s+the\s+test\s+file\s+aside\s+yields\s+a\s+passing\s+check\s+over\s+a\s+false\s+state\s+with\s+no\s+behaviou?ral\s+proof
            )$",
        )
        .expect("closed probe consequences")
    });
    let Some(action) = ACTION.captures(&decision.counterexample) else {
        return false;
    };
    // File operands must name a file, not 'nothing', or a multiword state.
    // The action grammar alone deliberately does not infer filenames.
    let file = action
        .name("moved")
        .or_else(|| action.name("renamed"))
        .expect("host action file")
        .as_str()
        .trim_matches(['`', '\'', '"']);
    if !(file.contains('.') || file.contains('/')) {
        return false;
    }
    let tail = &decision.counterexample[action.get(0).expect("host action").end()..];
    [tail, decision.reason.as_str()].into_iter().all(|text| {
        let clauses = CLAUSES
            .split(text)
            .map(str::trim)
            .filter(|clause| !clause.is_empty())
            .collect::<Vec<_>>();
        let mut clauses = clauses.into_iter().peekable();
        while let Some(clause) = clauses.next() {
            // Splitting 'and' in this cargo outcome leaves two dependent
            // fragments. Accept them only together, in their recorded order.
            if clause.eq_ignore_ascii_case("cargo test with --exact")
                && clauses
                    .peek()
                    .is_some_and(|next| next.eq_ignore_ascii_case("zero matches exits 0"))
            {
                clauses.next();
            } else if !CONSEQUENCE.is_match(clause) {
                return false;
            }
        }
        true
    })
}
