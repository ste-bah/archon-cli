use super::*;

fn assert_literal_operand(command: &'static str, valid: bool) {
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let set = frozen_set(&[("AC-X-001", "true", false)]);
        let root = set.project.path();
        std::fs::create_dir_all(root.join("key=../missing")).unwrap();
        std::fs::write(root.join("key=../missing/input"), "ready").unwrap();
        let mut failures = Vec::new();
        for cwd in [TrustedCwd::ProjectRoot, TrustedCwd::RepoRoot] {
            let actual = if valid {
                reader_path_decisions(&set, root, command, cwd)
            } else {
                decisions(&set, root, command, cwd)
            };
            if actual != [valid; 3] {
                failures.push(format!("{cwd:?} {command}: {actual:?}, expected {valid}"));
            }
            if !valid {
                let mut candidate = set.contract().acceptance[0].clone();
                candidate.check = AcceptanceCheck::Command {
                    command: command.into(),
                    cwd,
                };
                let defect = archon_workflow::task_set_contract::acceptance_site_path_defect(
                    &candidate, root, root, false,
                );
                if !defect.as_deref().is_some_and(|text| {
                    text.contains("../missing/mode=debug")
                        && text.contains("<site>/missing/mode=debug")
                        && text.contains("site does not create")
                }) {
                    failures.push(format!("{cwd:?} {command}: untrue diagnostic {defect:?}"));
                }
            }
        }
        sender.send(failures).unwrap();
    });
    let failures = receiver
        .recv_timeout(std::time::Duration::from_secs(60))
        .expect("site parity must finish");
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

fn reader_path_decisions(
    set: &super::super::super::super::republish::test_fixture::FrozenSet,
    root: &Path,
    command: &str,
    cwd: TrustedCwd,
) -> [bool; 3] {
    let mut contract = set.contract();
    contract.acceptance[0].check = AcceptanceCheck::Command {
        command: command.into(),
        cwd,
    };
    let strength =
        archon_workflow::verifier_strength::verifier_strength_defect(Some(command), None, None)
            .unwrap();
    let expected_message = format!("check 'AC-X-001': {strength}");
    let scope = AuthorScope::for_task_set(root, &set.tasks, &set.prd, false).unwrap();
    let reauthor = check_defects::check_defects(&contract, contract.acceptance[0].clone(), &scope);
    let text = std::fs::read_to_string(&set.prd).unwrap();
    let exact = contract
        .acceptance
        .iter()
        .map(|entry| (entry.id.clone(), entry.criterion.clone()))
        .collect();
    let expected = contract
        .acceptance
        .iter()
        .map(|entry| entry.id.clone())
        .collect();
    let first = super::super::super::super::preflight::prepare(
        root,
        &set.tasks,
        &set.prd,
        &content_digest(text.as_bytes()),
        &text,
        &exact,
        &expected,
        &serde_json::to_vec(&contract).unwrap(),
    );
    let first_only_strength = first
        .as_ref()
        .err()
        .and_then(|error| {
            error.downcast_ref::<crate::command::workflow_task_set_candidate::CandidateDefects>()
        })
        .is_some_and(|defects| {
            defects.0.len() == 1
                && defects.0[0].identity.code == "fixed_success_program"
                && defects.0[0].message == expected_message
        });
    let findings = super::super::super::super::findings::acceptance_findings(
        root,
        &set.prd,
        &text,
        &set.tasks.join(ACCEPTANCE_CONTRACT_FILE),
        &contract,
    )
    .unwrap();
    [
        reauthor.as_ref().err() == Some(&expected_message),
        first_only_strength,
        !findings.iter().any(|finding| {
            finding
                .deterministic_defect
                .as_ref()
                .is_some_and(|defect| defect.code == "unavailable_site_path")
        }),
    ]
}

#[test]
fn acceptance_site_selected_reader_operand_keeps_literal_equals() {
    assert_literal_operand("cat -e './key=../missing/input'", true);
}

#[test]
fn acceptance_site_selected_existence_operand_keeps_literal_equals() {
    assert_literal_operand("test -e ../missing/mode=debug", false);
}

#[test]
fn acceptance_site_selected_directory_operand_keeps_literal_equals() {
    assert_literal_operand("cd ../missing/mode=debug", false);
}

#[test]
fn acceptance_site_selected_stack_operand_keeps_literal_equals() {
    assert_literal_operand("pushd ../missing/mode=debug", false);
}
