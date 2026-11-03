use super::*;

/// The 17 real `TASK-*.md` files of `PRD-TRADING-DATA-LAKE-AHDM-001`,
/// checked in verbatim, and an expectation table transcribed from them by a
/// separate reader (see the generator note in the fixture directory). The
/// parser was previously only ever exercised against task files this
/// repository wrote itself — a closed loop in which a field the parser
/// ignored was also a field no test file declared.
/// The real decomposed-PRD corpus, which lives once at the workspace root
/// because several crates' tests parse the same seventeen task files. This
/// crate sits two levels below it, hence the `../..`: copying the corpus in
/// would let the two copies drift and the round-trip assertion would then
/// be checking this crate against itself.
fn fixture_root() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/prd-trading-data-lake-ahdm-001")
}

#[derive(serde::Deserialize)]
struct ExpectedContract {
    kind: String,
    artifact_path: String,
}

#[derive(serde::Deserialize)]
struct ExpectedTask {
    file: String,
    task_id: String,
    title: String,
    complexity: String,
    status: String,
    depends_on: Vec<String>,
    blocks: Vec<String>,
    required_env_keys: Vec<String>,
    required_tools: Vec<String>,
    deliverable_contracts: Vec<ExpectedContract>,
    acceptance_criteria: Vec<String>,
    files_expected_to_change: Vec<String>,
    files_forbidden_to_change: Vec<String>,
}

fn expected_tasks() -> Vec<ExpectedTask> {
    let path = fixture_root().join("expected-parse.json");
    let raw = fs::read_to_string(&path).expect("read expectation table");
    serde_json::from_str(&raw).expect("expectation table parses")
}

/// Every declared field of all 17 real task files, compared one field at a
/// time so a failure names the task and the field rather than diffing two
/// structs. Failures are collected, not short-circuited: the point is to
/// report *every* field that does not round-trip.
#[test]
fn every_declared_field_of_all_seventeen_real_tasks_round_trips() {
    let root = fixture_root();
    let expected = expected_tasks();
    assert_eq!(expected.len(), 17, "the PRD decomposes into 17 tasks");
    let mut failures = Vec::new();

    for want in &expected {
        let path = root.join(&want.file);
        let raw = fs::read_to_string(&path).expect("read fixture task file");
        let got = match parse_task_file(&path, &raw) {
            Ok(parsed) => parsed,
            Err(error) => {
                failures.push(format!("{}: parse failed: {error}", want.task_id));
                continue;
            }
        };
        let contracts = got
            .deliverable_contracts
            .iter()
            .map(|contract| (contract.kind.clone(), contract.artifact_path.clone()))
            .collect::<Vec<_>>();
        let want_contracts = want
            .deliverable_contracts
            .iter()
            .map(|contract| (contract.kind.clone(), contract.artifact_path.clone()))
            .collect::<Vec<_>>();
        #[rustfmt::skip]
        let fields: Vec<(&str, String, String)> = vec![
            ("task_id", got.canonical_task_id.clone(), want.task_id.clone()),
            ("title", format!("{:?}", got.title), format!("{:?}", Some(&want.title))),
            ("complexity", format!("{:?}", got.complexity), format!("{:?}", Some(&want.complexity))),
            ("status", format!("{:?}", got.status), format!("{:?}", Some(&want.status))),
            ("depends_on", format!("{:?}", got.dependency_ids), format!("{:?}", want.depends_on)),
            ("blocks", format!("{:?}", got.blocks_ids), format!("{:?}", want.blocks)),
            ("required_tools", format!("{:?}", got.required_tools), format!("{:?}", want.required_tools)),
            ("required_env_keys", format!("{:?}", got.required_env_keys), format!("{:?}", want.required_env_keys)),
            ("deliverable_contracts", format!("{contracts:?}"), format!("{want_contracts:?}")),
            ("acceptance_criteria", format!("{:?}", got.acceptance_criteria), format!("{:?}", want.acceptance_criteria)),
            ("files_expected_to_change", format!("{:?}", got.files_expected_to_change), format!("{:?}", want.files_expected_to_change)),
            ("files_forbidden_to_change", format!("{:?}", got.files_forbidden_to_change), format!("{:?}", want.files_forbidden_to_change)),
        ];
        for (field, parsed, declared) in fields {
            if parsed != declared {
                failures.push(format!(
                    "{} {field}:\n  parsed   {parsed}\n  declared {declared}",
                    want.task_id
                ));
            }
        }
    }

    assert!(
        failures.is_empty(),
        "fields that did not round-trip:\n{}",
        failures.join("\n")
    );
}

/// The richest deliverable contract in the real set — 30-odd verifier
/// fields. They deserialize into typed fields or they silently do not, and a
/// contract that lost its thresholds gates nothing.
#[test]
fn the_richest_deliverable_contract_keeps_its_verifier_fields() {
    let path = fixture_root().join("TASK-TDL-080-coverage-matrix-command.md");
    let raw = fs::read_to_string(&path).expect("read fixture");
    let parsed = parse_task_file(&path, &raw).expect("parse fixture");
    let contract = &parsed.deliverable_contracts[0];
    assert!(contract.required_universe);
    assert_eq!(contract.registry_minimum_count, 200);
    assert_eq!(contract.minimum_count_fields.get("row_count"), Some(&200));
    assert_eq!(contract.payload_format.as_deref(), Some("jsonl"));
    assert_eq!(contract.observed_time_field.as_deref(), Some("timestamp"));
    assert_eq!(contract.series_overlap_min_rows, 5);
    assert_eq!(
        contract
            .registry_identity_fields
            .get("canonical_instrument"),
        Some(&"symbol".to_string())
    );
    assert_eq!(
        contract.required_fields,
        ["timestamp", "open", "high", "low", "close", "volume"]
    );
    assert_eq!(contract.registry_allowed_statuses, ["Healthy"]);
}
