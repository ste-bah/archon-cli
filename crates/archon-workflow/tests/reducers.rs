use archon_workflow::{ReducerInput, ReducerKind, ReducerRegistry};

fn review(stage_id: &str, accepted: bool, content: &str) -> ReducerInput {
    ReducerInput {
        stage_id: stage_id.into(),
        content: content.into(),
        accepted,
        failed: !accepted,
    }
}

#[test]
fn each_reducer_deterministic() {
    let reducers = ReducerRegistry;
    let inputs = vec![
        review("r2", true, "approve: stable"),
        review("r1", true, "reject: blocking SQL injection"),
        review("r3", false, "timeout"),
    ];
    for kind in [
        ReducerKind::EvidenceWeightedReport,
        ReducerKind::ClaimVote,
        ReducerKind::AdversarialFindingsMerge,
        ReducerKind::CitationReconciliation,
        ReducerKind::CodeReviewSynthesis,
        ReducerKind::ChapterAssembly,
        ReducerKind::TaskDecomposition,
    ] {
        let first = reducers.reduce(kind, &inputs).unwrap();
        let second = reducers.reduce(kind, &inputs).unwrap();
        assert_eq!(first.body, second.body, "{kind:?} output must be stable");
    }
}

#[test]
fn dissent_included() {
    let output = ReducerRegistry
        .reduce(
            ReducerKind::ClaimVote,
            &[
                review("r1", true, "approve: looks fine"),
                review("r2", true, "approve: ok"),
                review("r3", true, "reject: SQL injection in auth.rs:42"),
            ],
        )
        .unwrap();
    assert!(
        output
            .dissent
            .iter()
            .any(|item| item.contains("SQL injection")),
        "{output:#?}"
    );
    assert!(output.body.contains("Dissent And Minority Findings"));
}

#[test]
fn failed_stage_summarized() {
    let output = ReducerRegistry
        .reduce(
            ReducerKind::EvidenceWeightedReport,
            &[
                review("ok", true, "accepted evidence"),
                review("bad", false, "rate limit exhausted"),
            ],
        )
        .unwrap();
    assert_eq!(output.failed_inputs, 1);
    assert!(output.body.contains("Failed `bad`: rate limit exhausted"));
}

#[test]
fn empty_input_no_fabrication() {
    let output = ReducerRegistry
        .reduce(ReducerKind::TaskDecomposition, &[])
        .unwrap();
    assert!(output.body.contains("No accepted inputs were available"));
    assert!(output.body.contains("No findings were fabricated"));
}

#[test]
fn citation_reconciliation_dedupes() {
    let output = ReducerRegistry
        .reduce(
            ReducerKind::CitationReconciliation,
            &[
                review("a", true, "https://example.com/a\nhttps://example.com/a"),
                review("b", true, "doi:10.123/example"),
            ],
        )
        .unwrap();
    assert_eq!(output.body.matches("https://example.com/a").count(), 1);
    assert!(output.body.contains("doi:10.123/example"));
}

#[test]
fn chapter_assembly_orders_stably() {
    let output = ReducerRegistry
        .reduce(
            ReducerKind::ChapterAssembly,
            &[review("b", true, "second"), review("a", true, "first")],
        )
        .unwrap();
    let first = output.body.find("Source `a`").unwrap();
    let second = output.body.find("Source `b`").unwrap();
    assert!(first < second, "chapters should follow stable stage order");
}
