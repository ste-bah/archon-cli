//! Issue-41: the PRD a task directory resolves to comes from the frozen
//! `acceptance-contract.json` first, and only then from the layout rules.
//!
//! The live failure: a set decomposed into `tasks/<id>-R2/` whose task bodies
//! carried no `prd:` line. The stem `<id>-R2` names no PRD, so the fidelity
//! gate reported "no PRD resolves" — while the contract beside the tasks held
//! the exact path and digest the decomposition had frozen.
//!
//! Fixtures are a made-up PRD in a made-up domain, deliberately.

use super::*;

const PRD_ID: &str = "PRD-LEDGER-007";

const PRD_BODY: &str = "## Goals\n\n| ID | Goal |\n|---|---|\n| G-LG-001 | Post every journal line once. |\n\n## Acceptance Criteria\n\n| ID | Criterion |\n|---|---|\n| AC-LG-001 | A posted line has a ledger entry. |\n";

/// A task set under `tasks/<PRD_ID>-R2/` — the re-decomposition suffix that
/// broke stem-based resolution — and its PRD under `prds/<PRD_ID>.md`.
///
/// `declare_prd` controls whether the task body carries a `prd:` line; the
/// fixed decomposition writes none, which is the case under test.
fn corpus(declare_prd: bool) -> (tempfile::TempDir, PathBuf, PathBuf) {
    let temp = tempfile::tempdir().expect("tempdir");
    let tasks = temp.path().join("tasks").join(format!("{PRD_ID}-R2"));
    fs::create_dir_all(&tasks).expect("tasks dir");
    let prds = temp.path().join(PRD_ROOT);
    fs::create_dir_all(&prds).expect("prds dir");
    let prd_path = prds.join(format!("{PRD_ID}.md"));
    fs::write(&prd_path, PRD_BODY).expect("prd");
    let prd_line = if declare_prd {
        format!("prd: {PRD_ID}\n")
    } else {
        String::new()
    };
    fs::write(
        tasks.join("TASK-LG-001.md"),
        format!(
            "# TASK-LG-001 — Post\n\n```yaml\ntask_id: TASK-LG-001\n{prd_line}title: Post\ncomplexity: medium\nstatus: pending\ndepends_on: []\nblocks: []\nimplements: [\"G-LG-001\", \"AC-LG-001\"]\nrequired_env_keys: []\nrequired_tools: [cargo]\ndeliverable_contracts: []\n```\n\n## Scope\n\nPost each line exactly once.\n"
        ),
    )
    .expect("task");
    (temp, tasks, prd_path)
}

/// The contract as the decomposition freezes it: the PRD path relative to the
/// project root and the digest `content_digest` computes over its bytes.
fn write_contract(tasks: &Path, digest: &str) {
    let contract = serde_json::json!({
        "schema_version": 1,
        "prd": { "path": format!("{PRD_ROOT}/{PRD_ID}.md"), "digest": digest },
        "gap_policy": { "permitted_acceptance_ids": [], "forbidden_phrases": [], "required_fields": [] },
        "acceptance": [],
        "supplementary": []
    });
    fs::write(
        tasks.join(ACCEPTANCE_CONTRACT_FILE),
        serde_json::to_vec_pretty(&contract).expect("contract json"),
    )
    .expect("contract");
}

fn claims(tasks: &Path) -> Vec<TaskRequirementClaims> {
    task_requirement_claims_tolerant(tasks).expect("claims").0
}

/// (a) The stem `<id>-R2` names nothing and no task declares `prd:`, yet the
/// contract's `prd.path` exists and digests as frozen: that file resolves.
#[test]
fn the_contract_prd_resolves_when_neither_stem_nor_task_names_one() {
    let (_temp, tasks, prd_path) = corpus(false);
    let digest = content_digest(&fs::read(&prd_path).expect("prd bytes"));
    write_contract(&tasks, &digest);

    let resolved = resolve_prd(&tasks, &claims(&tasks)).expect("no digest error");
    assert_eq!(resolved.as_deref(), Some(prd_path.as_path()));

    let report = section(Some(&tasks));
    assert!(
        report.contains("every obligation is claimed"),
        "the contracted PRD must drive the coverage report: {report}"
    );
    assert!(
        policy_findings(Some(&tasks)).is_empty(),
        "a clean set against its frozen PRD has no gate finding"
    );
}

/// (b) Without a contract, today's rules stand: the `-R2` stem resolves nothing
/// and the section skips; a task that declares `prd:` resolves by that id.
#[test]
fn without_a_contract_resolution_falls_back_to_the_layout_rules() {
    let (_temp, tasks, _prd_path) = corpus(false);
    assert_eq!(
        resolve_prd(&tasks, &claims(&tasks)).expect("no digest error"),
        None,
        "the -R2 stem names no PRD and no task declares one"
    );
    let report = section(Some(&tasks));
    assert!(
        report.contains("skipped") && report.contains(&format!("{PRD_ID}-R2.md")),
        "the skip must list the stem it tried: {report}"
    );

    let (_temp, tasks, prd_path) = corpus(true);
    assert_eq!(
        resolve_prd(&tasks, &claims(&tasks)).expect("no digest error"),
        Some(prd_path),
        "a declared `prd:` still resolves under prds/"
    );
}

/// (c) The contract names the PRD but the file no longer digests as frozen.
/// That is drift, and a guess would audit the wrong document: an error naming
/// both digests, surfaced by every entry point rather than a fall-through.
#[test]
fn a_contract_digest_mismatch_is_an_error_naming_both_digests() {
    let (_temp, tasks, prd_path) = corpus(false);
    let frozen = content_digest(&fs::read(&prd_path).expect("prd bytes"));
    write_contract(&tasks, &frozen);
    fs::write(
        &prd_path,
        format!("{PRD_BODY}| AC-LG-002 | Added after the freeze. |\n"),
    )
    .expect("drift the prd");
    let actual = content_digest(&fs::read(&prd_path).expect("drifted bytes"));
    assert_ne!(frozen, actual);

    let error = resolve_prd(&tasks, &claims(&tasks))
        .expect_err("a digest mismatch must not resolve")
        .to_string();
    assert!(
        error.contains(&frozen) && error.contains(&actual),
        "both digests must be named: {error}"
    );
    assert!(
        error.contains(ACCEPTANCE_CONTRACT_FILE) && error.contains(&format!("{PRD_ID}.md")),
        "the contract and the PRD must be named: {error}"
    );

    let report = section(Some(&tasks));
    assert!(
        report.contains(&frozen) && report.contains("skipped"),
        "the section must surface the mismatch, not a coverage result: {report}"
    );
    assert!(
        !report.contains("every obligation is claimed"),
        "a drifted PRD must not read as a pass: {report}"
    );

    let findings = policy_findings(Some(&tasks));
    assert_eq!(findings.len(), 1, "{findings:?}");
    assert!(
        findings[0].text.contains(&frozen) && findings[0].text.contains(&actual),
        "the gate finding names both digests: {}",
        findings[0].text
    );
    assert_eq!(
        findings[0].remediation_scope,
        archon_workflow::RemediationScope::PrdInput
    );
}

/// A contract whose `prd.path` is empty (the freeze-candidate placeholder) or
/// points at a file that does not exist is not a resolution and not an error:
/// the layout rules take over exactly as before.
#[test]
fn an_empty_or_missing_contract_path_falls_through() {
    let (_temp, tasks, prd_path) = corpus(true);
    for path in ["", "prds/PRD-LEDGER-999.md"] {
        let contract = serde_json::json!({
            "schema_version": 1,
            "prd": { "path": path, "digest": "" },
            "gap_policy": { "permitted_acceptance_ids": [], "forbidden_phrases": [], "required_fields": [] },
            "acceptance": [],
            "supplementary": []
        });
        fs::write(
            tasks.join(ACCEPTANCE_CONTRACT_FILE),
            serde_json::to_vec(&contract).expect("contract json"),
        )
        .expect("contract");
        assert_eq!(
            resolve_prd(&tasks, &claims(&tasks)).expect("no digest error"),
            Some(prd_path.clone()),
            "contract path {path:?} must fall through to the declared `prd:`"
        );
    }
}
