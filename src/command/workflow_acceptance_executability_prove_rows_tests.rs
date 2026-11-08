#![allow(non_snake_case)]
use super::super::{ExecutabilityProbe, HostProbe, probe_tests::trees};
use super::*;
use archon_workflow::task_set_contract::TrustedCwd;
use mutation::{CandidateDiagnostic, ExclusionReason::*};

pub(super) fn candidate(reason: mutation::ExclusionReason) -> CandidateDiagnostic {
    CandidateDiagnostic {
        message_mention: false,
        written_ancestor: false,
        path: "data/input".into(),
        move_path: None,
        move_root: None,
        exclusion: Some(reason),
    }
}
pub(super) fn pre(candidates: &[CandidateDiagnostic]) -> String {
    format!(
        "{}; considered inputs: {}",
        baseline_pass_finding_for_candidates("AC-X-001", "0123456789ab", candidates),
        describe_candidates(candidates)
    )
}
#[test]
fn P1_1_no_discovered_input() {
    let text = pre(&[]);
    assert!(text.starts_with("check 'AC-X-001': it passed on the pre-implementation tree at 0123456789ab, so it is not yet shown able to fail. Either it does not exercise what the work must add (make it fail on that tree), or the criterion already holds there and it must be proven a regression guard:"));
    assert!(text.contains("the command names no file that the host can move"));
    assert!(text.ends_with("considered inputs: none discovered"));
}
#[test]
fn P1_2_mechanics_root_tree() {
    let text = pre(&[
        candidate(CheckMechanics),
        candidate(RepositoryRoot),
        candidate(WholeDirectoryTree),
    ]);
    for reason in [CheckMechanics, RepositoryRoot, WholeDirectoryTree] {
        assert!(text.contains(reason.label()));
    }
    assert!(text.contains("the command names no file that the host can move"));
}
#[test]
fn P1_3_every_author_exclusion() {
    // MissingAtRoot is always eligible in this tree, so the brief's
    // no-eligible premise for that class is false. Exercise its real lane.
    let trees = trees(&[(
        "AC-X-001",
        "test -f data/missing.txt",
        TrustedCwd::ProjectRoot,
    )]);
    let contract = trees.contract();
    let discovery = mutation::input_discovery_for_check(
        &contract.acceptance[0],
        trees.set.project.path(),
        &trees.repo,
        None,
    );
    assert!(discovery.eligible.contains(&"data/missing.txt".into()));
    let text = pre(&[
        candidate(ManifestOrSetup),
        candidate(SiblingFromRoot),
        candidate(OutsideCopiedTree),
        candidate(CheckWritesPathInText),
    ]);
    for clause in [
        "manifests never move",
        "a sibling tree other than ../<repository dir>/ is never moved",
        "a path outside the copied tree",
        "a path the check itself writes",
    ] {
        assert!(text.contains(clause), "{text}");
    }
}
#[test]
fn P1_4_alias_only_is_host() {
    assert_eq!(
        super::super::routes::alias_only(&[
            candidate(CheckMechanics),
            candidate(RepositoryAliasUnverified)
        ])
        .as_deref(),
        Some("host could not verify the repository alias: data/input")
    );
}
#[test]
fn P1_5_alias_mixed_with_author_exclusion() {
    let candidates = [
        candidate(RepositoryAliasUnverified),
        candidate(ManifestOrSetup),
    ];
    assert!(super::super::routes::alias_only(&candidates).is_none());
    let text = pre(&candidates);
    assert!(text.contains("manifests never move"));
    assert!(text.contains("the host could not verify `data/input` as the repository alias"));
}

/// All post-run rows inject the exact observation read by take_prove_run;
/// neither the command nor a compiler is launched by these fixtures.
pub(super) async fn injected(
    command: &str,
    code: i32,
    moved: bool,
    state: &str,
    stderr: &str,
    generated: bool,
    recreated: bool,
) -> (
    BTreeMap<String, String>,
    BTreeMap<String, (String, UnprovenCause)>,
    BTreeMap<String, String>,
) {
    let trees = trees(&[("AC-X-001", command, TrustedCwd::ProjectRoot)]);
    let contract = trees.contract();
    let probe = HostProbe::for_task_set(trees.set.project.path(), &trees.set.tasks);
    let markers = mutation::Markers::new();
    let n = markers.nonce();
    let mut err = stderr.to_string();
    if moved {
        err.push_str(&format!("\narchon-mutation-moved-{n}: 0\n"));
    }
    if generated {
        err.push_str(&format!("archon-mutation-check-recreated-{n}: 0\n"));
    }
    let result = archon_workflow::acceptance_scratch::CheckResult {
        acceptance_id: "AC-X-001".into(), exit_code: Some(code),
        stdout: format!("archon-mutation-state-{n}: 0:{state}\narchon-mutation-status-{n}: {}:{code}:none:none\n", markers.tag()).into_bytes(),
        stderr: format!("{err}\narchon-mutation-move-state-{n}: 0:{}\n", if moved { "moved" } else { state }).into_bytes(),
        quota_walk_count: 0, environment_note: None, operational_error: None, fault: None, classification: None, verdict_streams: Default::default()
    };
    let mut result = result;
    if recreated {
        result
            .stderr
            .extend_from_slice(format!("archon-mutation-recreated-{n}: 0\n").as_bytes());
    }
    let evidence = super::super::hermetic::CheckEvidence {
        result,
        copy_layout: None,
    };
    probe.inject_prove_run(
        super::super::repairs::TreeRun::known(&[evidence], "fixture"),
        markers,
    );
    let digest = super::super::super::contract_digest(&contract).unwrap();
    let findings = super::super::prove(
        &probe,
        &Baseline {
            origin: crate::command::workflow_task_set::executability::BaselineOrigin::RecordedBase,
            commit: trees.base,
            repository: trees.repo,
        },
        &contract,
        &digest,
        &[],
        &["AC-X-001".into()],
        &BTreeMap::new(),
    )
    .await;
    for text in findings.values() {
        HostProbe::assert_p1_readers(text, true);
    }
    (
        findings,
        probe.take_unproven_causes(),
        probe.take_regression_guards(),
    )
}
async fn unmoved(
    state: &str,
) -> (
    BTreeMap<String, String>,
    BTreeMap<String, (String, UnprovenCause)>,
    BTreeMap<String, String>,
) {
    injected("test -f data/state.txt", 0, false, state, "", false, false).await
}
#[tokio::test]
async fn P1_6_absent_input() {
    let (f, u, _) = unmoved("absent at the selected root").await;
    assert!(u.is_empty());
    assert!(
        f["AC-X-001"].contains("`data/state.txt` does not exist in the pre-implementation commit")
    );
    assert!(f["AC-X-001"].contains("name a file that exists there"));
    assert!(!f["AC-X-001"].contains("eligible movable input"));
    assert!(!f["AC-X-001"].contains("names no file"));
}
#[tokio::test]
async fn P1_7_move_io_failure_is_host() {
    let (f, u, _) = unmoved("move failed: permission denied").await;
    assert!(f.is_empty());
    assert_eq!(u["AC-X-001"].1, UnprovenCause::Host);
}
#[tokio::test]
async fn P1_8_symlink_remedy() {
    let (f, u, _) = unmoved("refused: symlink component").await;
    assert!(u.is_empty());
    assert!(f["AC-X-001"].contains(
        "`data/state.txt` reaches its target through a symbolic link; name the real path"
    ));
}
#[test]
fn P1_9_mixed_states_host_wins_otherwise_both_remedies() {
    let trees = trees(&[(
        "AC-X-001",
        "test -f data/state.txt",
        TrustedCwd::ProjectRoot,
    )]);
    let probe = HostProbe::for_task_set(trees.set.project.path(), &trees.set.tasks);
    let baseline = Baseline {
        origin: crate::command::workflow_task_set::executability::BaselineOrigin::RecordedBase,
        commit: trees.base.clone(),
        repository: trees.repo.clone(),
    };
    let mut states = BTreeMap::from([
        ("a".into(), "absent at the selected root".into()),
        ("b".into(), "refused: symlink component".into()),
    ]);
    let f = super::super::routes::no_moves(
        &probe,
        &baseline,
        &trees.contract(),
        "AC-X-001",
        "base",
        &states,
        "lead",
    )
    .unwrap();
    assert!(f.contains("`a` does not exist"));
    assert!(f.contains("`b` reaches its target"));
    states.insert("c".into(), "move failed: I/O".into());
    for _ in 0..2 {
        assert!(
            super::super::routes::no_moves(
                &probe,
                &baseline,
                &trees.contract(),
                "AC-X-001",
                "base",
                &states,
                "lead"
            )
            .is_none()
        );
    }
    assert_eq!(
        probe.take_unproven_causes()["AC-X-001"].1,
        UnprovenCause::PermanentHost
    );
}
#[tokio::test]
async fn P1_10_still_passed_without_bound_judge() {
    let (f, u, _) = injected(
        "cat data/state.txt; true",
        0,
        true,
        "restored",
        "",
        false,
        false,
    )
    .await;
    assert!(u.is_empty());
    assert!(f["AC-X-001"].contains(
        "it still passed with data/state.txt moved aside, so it does not depend on them"
    ));
}
#[tokio::test]
async fn P1_11_head_is_not_preimplementation() {
    let trees = trees(&[("AC-X-001", "true", TrustedCwd::ProjectRoot)]);
    assert_ne!(trees.base, trees.head);
    let probe = HostProbe::for_task_set(trees.set.project.path(), &trees.set.tasks);
    // Reverification's supplied tree and copy layout must stay together.
    // P2 owns choosing the baseline for the reverify proof entry point.
    let contract = trees.contract();
    let findings = super::super::prove(
        &probe,
        &Baseline {
            origin: crate::command::workflow_task_set::executability::BaselineOrigin::ProbedTree,
            commit: trees.head.clone(),
            repository: trees.repo.clone(),
        },
        &contract,
        &super::super::super::contract_digest(&contract).unwrap(),
        &[],
        &["AC-X-001".into()],
        &BTreeMap::new(),
    )
    .await;
    let text = &findings["AC-X-001"];
    let (lead, _) = text.split_once(super::super::super::CANNOT_FAIL).unwrap();
    assert!(!lead.contains("pre-implementation"), "{text}");
    assert!(
        text.contains(
            "The check must fail on the pre-implementation tree, or be proven a regression guard"
        ),
        "{text}"
    );
    assert!(text.contains("passed on the probed tree"), "{text}");
    assert!(text.contains(&trees.head[..12]), "{text}");
    HostProbe::assert_p1_readers(text, true);
}
#[tokio::test]
async fn P1_20_check_recreation_is_observed() {
    let (f, _, _) = injected(
        "test -f data/state.txt",
        0,
        true,
        "original-recreated",
        "",
        true,
        true,
    )
    .await;
    assert!(f["AC-X-001"].contains("it recreated data/state.txt while running"));
}
#[tokio::test]
async fn P1_21_parent_touch_has_remedy() {
    let (f, _, _) = injected(
        "test -f data/state.txt",
        0,
        true,
        "original-recreated",
        "",
        false,
        true,
    )
    .await;
    assert!(
        f["AC-X-001"]
            .contains("do not create, replace or link files beside data/state.txt while it runs")
    );
}
#[tokio::test]
async fn P1_22_snapshot_failure_is_host() {
    let (f, u, _) = injected(
        "test -f data/state.txt",
        0,
        true,
        "unsafe-to-restore: parent snapshot failed: I/O",
        "",
        false,
        false,
    )
    .await;
    assert!(f.is_empty());
    assert_eq!(u["AC-X-001"].1, UnprovenCause::PermanentHost);
}
#[tokio::test]
async fn P1_23_compile_location_is_moved() {
    let (f, u, guards) = injected(
        "cargo test --lib x; grep -q fn data/state.txt",
        101,
        true,
        "restored",
        "data/state.txt:1: error: missing input",
        false,
        false,
    )
    .await;
    assert!(f.is_empty() && u.is_empty());
    assert!(guards.contains_key("AC-X-001"));
}
#[tokio::test]
async fn P1_24_module_error_names_moved_input() {
    let fixture = tempfile::tempdir().unwrap();
    std::fs::create_dir(fixture.path().join("src")).unwrap();
    std::fs::write(
        fixture.path().join("Cargo.toml"),
        "[package]\nname = \"mutation-fixture\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
    )
    .unwrap();
    std::fs::write(
        fixture.path().join("src/lib.rs"),
        "mod app;\n#[test] fn x() { app::f(); }\n",
    )
    .unwrap();
    std::fs::write(fixture.path().join("src/app.rs"), "pub fn f() {}\n").unwrap();
    std::fs::rename(
        fixture.path().join("src/app.rs"),
        fixture.path().join("src/app.aside"),
    )
    .unwrap();
    std::fs::write(
        fixture.path().join("Cargo.lock"),
        "version = 4\n\n[[package]]\nname = \"mutation-fixture\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    // The compile runs through a hermetic stub, never the operator's cargo.
    // grep supplies the real shell status (2), after the missing-module build.
    let stub = fixture.path().join("cargo-stub.sh");
    std::fs::write(&stub, "#!/bin/sh\nprintf 'error[E0583]: file not found for module `app`\\n --> src/lib.rs:1:1\\n' >&2\nexit 101\n").unwrap();
    let diagnostic = std::process::Command::new("sh")
        .args([
            "-c",
            "sh ./cargo-stub.sh test --lib x -- --exact; grep -q fn src/app.rs",
        ])
        .current_dir(fixture.path())
        .output()
        .unwrap();
    assert_eq!(diagnostic.status.code(), Some(2));
    let (f, u, guards) = injected(
        "cargo test --lib x -- --exact; grep -q fn src/app.rs",
        2,
        true,
        "restored",
        &String::from_utf8_lossy(&diagnostic.stderr),
        false,
        false,
    )
    .await;
    assert!(f.is_empty() && u.is_empty(), "{f:?} {u:?}");
    assert!(guards.contains_key("AC-X-001"));
}
#[tokio::test]
async fn P1_25_unrelated_compile_error_keeps_rule_four() {
    let (f, u, guards) = injected(
        "cargo test --lib x; grep -q fn data/state.txt",
        101,
        true,
        "restored",
        "src/unrelated.rs:1: error: mismatched types",
        false,
        false,
    )
    .await;
    assert!(guards.is_empty());
    assert!(f.is_empty());
    assert!(u["AC-X-001"].0.contains("did not build"));
    assert_eq!(u["AC-X-001"].1, UnprovenCause::PermanentHost);
}
#[tokio::test]
async fn P1_27_build_is_assertion() {
    let (f, u, guards) = injected(
        "test -f data/state.txt; cargo build",
        101,
        true,
        "restored",
        "src/unrelated.rs:1: error: mismatched types",
        false,
        false,
    )
    .await;
    assert!(f.is_empty() && u.is_empty());
    assert!(guards.contains_key("AC-X-001"));
}
#[tokio::test]
async fn P1_28_script_crash_has_specific_defect() {
    let (f, _, _) = injected("test -f data/state.txt; python3 -c 'undefined_name()'", 1, true, "restored", "Traceback (most recent call last):\n  File \"<string>\", line 1, in <module>\nNameError: name 'undefined_name' is not defined", false, false).await;
    let text = &f["AC-X-001"];
    assert!(text.contains("undefined_name"));
    assert!(text.contains("fix that defect"));
}
#[tokio::test]
async fn P1_29_missing_program_has_cause_and_remedy() {
    for (cause, remedy) in [
        (
            UnprovenCause::SiteLacks,
            "without depending on that program or subcommand",
        ),
        (
            UnprovenCause::ProgramAbsent,
            "check the criterion with what the site provides: make it fail by its own assertion on the pre-implementation tree, or prove it a regression guard.",
        ),
    ] {
        let reason = super::super::helpers::mutation_defect_reason(
            &Some(("specific cause".into(), cause)),
            String::new(),
        );
        assert!(reason.contains("specific cause"));
        assert!(reason.contains(remedy));
    }
    let (f, _, _) = injected(
        "test -f data/state.txt; archon_p1_absent_program",
        127,
        true,
        "restored",
        "sh: archon_p1_absent_program: command not found",
        false,
        false,
    )
    .await;
    let text = &f["AC-X-001"];
    assert!(text.contains("archon_p1_absent_program"));
    assert!(text.contains("what the site provides"));
}

#[test]
fn shared_move_aside_handle_sentence_matches_author_rule() {
    const S1: &str = "A path the check reads by its literal relative path, as an operand of a command that reads it (an existence test included), is a move-aside handle: the host may move it aside to prove the check can fail. A handle is not the deciding assertion; the test or implementation run whose status decides the check must still decide it.";
    assert!(baseline_pass_finding_without_discovered_input("id", "base").contains(S1));
    // P4 owns asserting the shared sentence in its check_path_rule.
}
