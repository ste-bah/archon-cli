use super::*;

const R18: &str = include_str!("../../../../tests/fixtures/r18_plan/prd.md");

#[test]
fn path_rule_table() {
    for text in [
        "`x`",
        "run `anything at all`",
        "``x``",
        "/root",
        "\\root",
        "./item",
        "../item",
        "~/item",
        "C:\\item",
        "d:/item",
        "Z:\\",
        "a/b.txt",
        "a\\b.X1",
        "a/b.abcdefghij",
        "([\"a/b.txt\"])!",
        "('/root'),",
        "[C:\\item];",
        "a/é.x",
    ] {
        assert!(names_path_command_or_file(text), "{text}");
    }
    for text in [
        "changed/new",
        "Derived/resampled",
        "4H / 240",
        "e.g.",
        "0.95",
        "v1.2",
        "a.toml",
        "`",
        "``",
        "a/.txt",
        "a/b.123",
        "a/b.abcdefghijk",
        "a/b.x-y",
        "a/b.é",
        "a/b.txt/last",
        "ordinary words",
        "4H \\ 240",
    ] {
        assert!(!names_path_command_or_file(text), "{text}");
    }
    assert_eq!(INSPECTION_LOGIC, 1);
}

#[test]
fn r18_eligibility_is_asserted_per_id() {
    let eligible = inspection_eligible(R18);
    println!(
        "R18 eligible ids: {}",
        eligible.keys().cloned().collect::<Vec<_>>().join(", ")
    );
    let expected = [
        "NFR-001", "NFR-002", "NFR-003", "NFR-004", "NFR-005", "NFR-006", "RD-002", "RD-003",
        "RD-004", "RD-005", "RD-006", "RD-007", "RD-008", "TF-001", "TF-002", "TF-003", "TF-004",
        "TF-005",
    ];
    assert_eq!(
        eligible.keys().map(String::as_str).collect::<Vec<_>>(),
        expected
    );
    for (id, text) in crate::obligation_ids::constraint_texts(R18) {
        assert_eq!(
            eligible.contains_key(&id),
            expected.contains(&id.as_str()),
            "{id}: {text}"
        );
        if !text.contains('`') {
            assert!(eligible.contains_key(&id), "{id}");
        }
    }
    assert!(!eligible.contains_key("RD-001"));
    for id in crate::obligation_ids::obligation_texts(R18).keys() {
        assert!(!eligible.contains_key(id), "obligation {id}");
    }
    for id in crate::obligation_ids::acceptance_ids(R18) {
        assert!(!eligible.contains_key(&id), "acceptance {id}");
    }
}

#[test]
fn eligibility_keeps_only_constraints_without_named_artifacts() {
    for text in [
        "Keep every record.",
        "Preserve changed/new evidence.",
        "Keep 0.95 as the threshold.",
    ] {
        let prd = format!(
            "## Constraints\n- NFR-001: {text}\n\n## Goals\n| ID | Goal |\n|---|---|\n| G-001 | {text} |\n\n## Acceptance\n| ID | Criterion |\n|---|---|\n| AC-X-001 | {text} |\n\n## Done Definition\n1. {text}\n"
        );
        assert_eq!(
            inspection_eligible(&prd),
            BTreeMap::from([("NFR-001".into(), text.into())])
        );
        for named in ["`record`", "/records", "records/item.txt"] {
            assert!(inspection_eligible(&prd.replace(text, named)).is_empty());
        }
    }
}

#[test]
fn items_remove_only_covered_ids_from_both_contract_lists() {
    let fixture: AcceptanceContract = serde_json::from_str(include_str!(
        "../../../../tests/fixtures/r18_plan/acceptance-contract.json"
    ))
    .unwrap();
    for id in ["NFR-001", "RD-002", "TF-003"] {
        for supplementary in [false, true] {
            let mut contract = fixture.clone();
            let mut entry = contract.acceptance[0].clone();
            entry.covers = vec![format!(" {id} ")];
            contract.acceptance.clear();
            contract.supplementary.clear();
            if supplementary {
                contract.supplementary.push(entry);
            } else {
                contract.acceptance.push(entry);
            }
            let mut expected = inspection_eligible(R18);
            expected.remove(id);
            assert_eq!(inspection_items(R18, &contract), expected);
        }
    }
}

fn collapse(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn assert_checklist(text: &str, expected: &[&str]) {
    let questions = checklist(text);
    assert_eq!(
        questions
            .iter()
            .map(|q| q.statement.as_str())
            .collect::<Vec<_>>(),
        expected
    );
    assert_invariant(text);
}

fn assert_invariant(text: &str) {
    let questions = checklist(text);
    assert!((1..=8).contains(&questions.len()));
    for (index, question) in questions.iter().enumerate() {
        assert_eq!(question.number, index as u32 + 1);
    }
    let joined = questions
        .iter()
        .map(|q| q.statement.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    assert_eq!(collapse(&joined), collapse(text));
}

#[test]
fn checklist_respects_protected_text_and_separators() {
    for text in [
        "Keep e.g. these examples. Preserve every source.",
        "Keep a 0.95 threshold. Preserve every source.",
        "Keep the `a.b` token. Preserve every source.",
        "Keep (all examples. Even these!) intact. Preserve every source.",
        "Keep [all examples; even these?] intact. Preserve every source.",
        "Keep the `a. b; c! d?` token. Preserve every source.",
        "Keep letter A. with context. Preserve every source.",
    ] {
        let (first, second) = text.rsplit_once(" Preserve").unwrap();
        assert_checklist(text, &[first, &format!("Preserve{second}")]);
    }
    assert_checklist(
        "Keep every record; retain every quote! Verify every result?",
        &[
            "Keep every record;",
            "retain every quote!",
            "Verify every result?",
        ],
    );
}

#[test]
fn checklist_merges_short_statements_and_caps_questions() {
    assert_checklist("Keep it.", &["Keep it."]);
    assert_checklist(
        "Yes. Keep every record. Preserve all evidence.",
        &["Yes. Keep every record.", "Preserve all evidence."],
    );
    assert_checklist(
        "Keep every record. Yes! Preserve all evidence.",
        &["Keep every record. Yes!", "Preserve all evidence."],
    );
    assert_checklist("", &[""]);
    assert_checklist(" \n\t ", &[""]);
    let sentences: Vec<_> = (1..=12)
        .map(|n| format!("Keep record number {n}."))
        .collect();
    let text = sentences.join(" ");
    let tail = sentences[7..].join(" ");
    let mut expected: Vec<_> = sentences[..7].iter().map(String::as_str).collect();
    expected.push(&tail);
    assert_checklist(&text, &expected);
}

#[test]
fn checklist_invariant_on_every_requirement_in_both_fixtures() {
    for prd in [
        R18,
        include_str!("../../../../tests/fixtures/offline_decomposition/prd.md"),
    ] {
        for text in crate::obligation_ids::obligation_texts(prd)
            .into_values()
            .chain(crate::obligation_ids::constraint_texts(prd).into_values())
        {
            assert_invariant(&text);
        }
    }
}
