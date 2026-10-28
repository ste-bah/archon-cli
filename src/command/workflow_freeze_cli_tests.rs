//! Staged-path invariants for `workflow freeze-*` (child module via #[path];
//! file-size guard).

use super::*;

#[test]
fn staged_client_construction_and_both_freezes_report_operational_faults() {
    let whole = include_str!("workflow_freeze_cli.rs");
    let source = &whole[..whole.find("#[cfg(test)]").expect("test module marker")];
    for command in ["freeze-acceptance", "freeze-skeleton"] {
        let reported = source
            .split("return report_operational_failure(")
            .skip(1)
            .any(|block| block[..block.len().min(120)].contains(command));
        assert!(
            reported,
            "{command} must surface an operational failure as the envelope's reason"
        );
    }
    assert_eq!(
        source.matches("return report_operational_failure(").count(),
        3,
        "client construction and both staged paths report operationally rather than exiting non-zero"
    );
}

#[test]
fn authored_incomplete_freeze_stages_the_script_operational_envelope() {
    let temp = tempfile::tempdir().unwrap();
    let staging = temp.path().join("stage");
    std::fs::create_dir_all(&staging).unwrap();
    let envelope = staging.join("gate-envelope.json");
    let incomplete = crate::command::workflow_freeze_budget::FreezeIncomplete::stalled(
        "unusable authored reply",
        &crate::command::workflow_freeze_budget::FreezeProgress::default(),
    )
    .authored_judge_reply();
    report_authored_judge_incomplete(
        temp.path(),
        StagedArgs {
            staging_root: &staging,
            gate_envelope: &envelope,
            call_id: "call-1",
        },
        "freeze-acceptance",
        &incomplete,
    )
    .unwrap();
    let value: serde_json::Value =
        serde_json::from_slice(&std::fs::read(envelope).unwrap()).unwrap();
    assert_eq!(value["operational_error"]["kind"], "authored_judge_reply");
    assert!(
        value["operational_error"]["text"]
            .as_str()
            .unwrap()
            .contains("unusable authored reply")
    );
}

#[test]
fn both_json_staged_paths_refuse_the_candidate_instead_of_failing_the_run() {
    let whole = include_str!("workflow_freeze_cli.rs");
    // Only the production half counts: this module's own literals would
    // otherwise satisfy the assertion about the code it is checking.
    let source = &whole[..whole.find("#[cfg(test)]").expect("test module marker")];
    for (command, gate) in [
        ("freeze-acceptance", "GateId::FreezeAcceptance"),
        ("freeze-skeleton", "GateId::FreezeSkeleton"),
    ] {
        let refused = source
            .split("return refuse_candidate_artifact(")
            .skip(1)
            .any(|block| {
                let head = &block[..block.len().min(400)];
                head.contains(command) && head.contains(gate)
            });
        assert!(
            refused,
            "{command} must refuse a malformed candidate through the findings channel"
        );
    }
    // Acceptance refuses twice — once for a candidate that will not parse and
    // once for one the freeze itself rejects — so the total is a floor, not
    // a fixed number. What must hold is that no candidate problem leaves by
    // any other exit.
    assert!(
        source.matches("return refuse_candidate_artifact(").count() >= 2,
        "every JSON staged path routes candidate problems through the findings channel"
    );
}

/// A staged freeze must leave its findings readable on disk.
///
/// The staged path is what the decomposition drives through `hostCommand`, and
/// unlike the interactive path it never calls `run_sync_gate`, so nothing wrote
/// shadow records for it. The lock a staged freeze publishes carries only
/// `finding_count` and `findings_digest`, so a run could report "1 finding" and
/// make it permanently unreadable — which is exactly what happened to the
/// acceptance finding that a week of proof runs was then built on top of.
#[test]
fn a_staged_freeze_records_its_findings_where_a_human_can_read_them() {
    let temp = tempfile::tempdir().expect("tempdir");
    let cwd = temp.path();
    let staging_root = cwd.join("staging");
    std::fs::create_dir_all(&staging_root).expect("staging root");
    let gate_envelope = staging_root.join("envelope.json");

    let finding = crate::command::workflow_gate::GateFinding::new(
        crate::command::workflow_gate::GateId::FreezeAcceptance,
        "check 'AC-X-001' floor is not falsifiable: deliverable contract has neither a verifier nor a positive instance obligation",
        "AC-X-001",
        None,
        archon_workflow::RemediationScope::InheritedPredecessor,
    );
    let evaluation = crate::command::workflow_gate::GateEvaluation::new("staged", vec![finding]);

    write_staged_manifest(
        cwd,
        StagedArgs {
            staging_root: &staging_root,
            gate_envelope: &gate_envelope,
            call_id: "call-1",
        },
        "freeze-acceptance",
        evaluation,
        Vec::new(),
    )
    .expect("staged manifest");

    // The staged child publishes nothing live: it may only prepare, and the
    // parent still refuses on non-zero exit, digest mismatch or sentinel
    // violation. The finding text therefore has to survive in the envelope the
    // parent reads, which is also what reaches `.decompose.log`.
    let text = std::fs::read_to_string(&gate_envelope).unwrap_or_else(|error| {
        panic!(
            "a staged freeze must write its findings to {}: {error}",
            gate_envelope.display()
        )
    });
    assert!(
        text.contains("floor is not falsifiable"),
        "the envelope must carry the finding text, not just a count: {text}"
    );
    assert!(
        text.contains("AC-X-001"),
        "the envelope must name the subject the finding is about: {text}"
    );
    // And it must not have written live state itself.
    let log = crate::command::workflow_gate::shadow_log_path(cwd);
    assert!(
        !log.exists(),
        "the staged child must not append to the live shadow log at {}; the parent records findings only after it commits",
        log.display()
    );
}

/// `freeze-acceptance --reauthor --prd "" ...` fails at the command line and
/// names the option. Before, the empty value resolved to the working
/// directory and failed later as "not an absolute path".
#[test]
fn an_empty_or_missing_cli_path_fails_naming_its_option() {
    let cwd = tempfile::tempdir().unwrap();
    let error = required_path(cwd.path(), Path::new(""), "--prd").expect_err("empty");
    assert_eq!(
        error.to_string(),
        "--prd is empty; it must name an existing path"
    );
    let error = required_path(cwd.path(), Path::new("  "), "--tasks").expect_err("blank");
    assert!(error.to_string().starts_with("--tasks is empty"), "{error}");
    let error = required_path(cwd.path(), Path::new("prds/none.md"), "--prd").expect_err("missing");
    assert!(
        error.to_string().starts_with("--prd names ")
            && error.to_string().ends_with("which does not exist"),
        "{error}"
    );
    std::fs::write(cwd.path().join("spec.md"), "x").unwrap();
    assert_eq!(
        required_path(cwd.path(), Path::new("spec.md"), "--prd").unwrap(),
        cwd.path().join("spec.md")
    );
}

/// Issue 255: an incomplete, resumable freeze leaves by the host's
/// operational exit, never through the envelope or a plain failure -- except
/// an unusable judge reply the model authored (F3), which the envelope carries
/// back to the script's retry (`an_authored_judge_incomplete_staged_freeze_...`).
#[test]
fn an_incomplete_freeze_exits_by_the_operational_contract() {
    let whole = include_str!("workflow_freeze_cli.rs");
    let source = &whole[..whole.find("#[cfg(test)]").expect("test module marker")];
    let branch = source
        .split("FreezeIncomplete::caused(&error)")
        .nth(1)
        .expect("the staged acceptance freeze tells an incomplete freeze apart");
    let branch = &branch[..branch.find("\n                }\n").expect("its end")];
    let authored = branch
        .split("if incomplete.is_authored_judge_reply() {")
        .nth(1)
        .expect("only an authored judge reply is told apart");
    let (authored, rest) = authored
        .split_once("\n                    }\n")
        .expect("its block");
    assert!(
        authored
            .trim_start()
            .starts_with("return report_authored_judge_incomplete("),
        "{authored}"
    );
    assert_eq!(
        rest.trim(),
        "exit_incomplete_resumable(incomplete);",
        "every other incomplete freeze exits resumable: {branch}"
    );
    let exit = source
        .split("fn exit_incomplete_resumable(")
        .nth(1)
        .expect("the exit");
    let body = &exit[..exit.find("\n}\n").expect("its end")];
    assert!(body.contains("incomplete.report()") && body.contains("EXIT_INCOMPLETE_RESUMABLE"));
    assert_eq!(
        crate::command::workflow_host_command_operational::EXIT_INCOMPLETE_RESUMABLE,
        75
    );
}

#[test]
fn workflow_host_command_candidate_refusal_has_host_owned_stable_identity() {
    let dir = tempfile::tempdir().expect("fixture");
    let mut identities = Vec::new();
    for ordinal in 1..=3 {
        let staging = dir.path().join(format!("stage-{ordinal}"));
        std::fs::create_dir_all(&staging).expect("staging");
        let envelope = staging.join("envelope.json");
        refuse_candidate_artifact(
            dir.path(),
            StagedArgs {
                staging_root: &staging,
                gate_envelope: &envelope,
                call_id: "call",
            },
            "freeze-skeleton",
            crate::command::workflow_gate::GateId::FreezeSkeleton,
            "skeleton",
            "candidate_refused",
            &format!("invalid submitted filename bad{ordinal}"),
        )
        .expect("refusal envelope");
        let value: serde_json::Value =
            serde_json::from_slice(&std::fs::read(envelope).expect("bytes")).expect("envelope");
        let identity = value["policy_findings"][0]["deterministic_defect"].clone();
        assert_eq!(identity["provenance"], "host_validator", "{value}");
        assert!(
            identity["code"]
                .as_str()
                .is_some_and(|code| !code.is_empty()),
            "{value}"
        );
        assert!(
            !identity.to_string().contains(&format!("bad{ordinal}")),
            "{identity}"
        );
        identities.push(identity);
    }
    assert!(identities.windows(2).all(|pair| pair[0] == pair[1]));
}

#[test]
fn workflow_host_command_reports_all_marker_fields_in_one_task() {
    let value = serde_json::json!({"tasks":[{"task_id":"TASK-X-001", "file_name":"<redacted>",
        "depends_on":[{"consumes":[{"artifact_path":"<redacted>"}]}],
        "deliverable_contracts":[{"artifact_path":"<redacted>"}]}]});
    let message = crate::command::workflow_freeze_candidate::skeleton_marker_refusal(&value)
        .expect("markers");
    for field in ["file_name", "depends_on", "deliverable_contracts"] {
        assert!(message.contains(field), "missing {field}: {message}");
    }
}

/// Issue 261 round 7: a candidate of the wrong shape was parsed. Its refusal
/// keeps the shape stage (refused tier), below which only unreadable JSON sits.
#[test]
fn workflow_host_command_shape_and_json_refusals_carry_their_own_stage() {
    #[derive(serde::Deserialize)]
    #[allow(dead_code)]
    struct Wanted {
        tasks: Vec<String>,
    }
    let dir = tempfile::tempdir().expect("fixture");
    for (raw, code, stage) in [
        (&b"{\"other\":1}"[..], "invalid_candidate_shape", "shape"),
        (&b"no json here"[..], "invalid_json", "parse"),
    ] {
        let (found, reason) =
            crate::command::workflow_freeze_candidate::candidate_refusal::<Wanted>(raw)
                .expect("refused");
        assert_eq!(found, code);
        let staging = dir.path().join(code);
        std::fs::create_dir_all(&staging).expect("staging");
        let envelope = staging.join("envelope.json");
        refuse_candidate_artifact(
            dir.path(),
            StagedArgs {
                staging_root: &staging,
                gate_envelope: &envelope,
                call_id: "call",
            },
            "freeze-skeleton",
            crate::command::workflow_gate::GateId::FreezeSkeleton,
            "skeleton",
            found,
            &reason,
        )
        .expect("refusal envelope");
        let value: serde_json::Value =
            serde_json::from_slice(&std::fs::read(envelope).expect("bytes")).expect("envelope");
        let defect = &value["policy_findings"][0]["deterministic_defect"];
        assert_eq!(defect["code"], code, "{value}");
        assert_eq!(defect["stage"], stage, "{value}");
    }
}

/// F3 through the real staged stage: a judge whose every reply is unusable
/// JSON it authored ends the staged freeze through the gate envelope (kind
/// `authored_judge_reply`), so the script retries it in its ordinary
/// no-progress window; it never exits resumable without an envelope. The
/// stage runs in a child process, so that an exit is observed, not suffered.
#[cfg(unix)]
#[tokio::test]
async fn an_authored_judge_incomplete_staged_freeze_writes_its_envelope_and_returns() {
    const CHILD: &str = "ARCHON_TEST_STAGED_FREEZE_DIR";
    let Some(dir) = std::env::var_os(CHILD).map(PathBuf::from) else {
        let temp = tempfile::tempdir().unwrap();
        let name = concat!(
            module_path!(),
            "::an_authored_judge_incomplete_staged_freeze_writes_its_envelope_and_returns"
        );
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", name.split_once("::").unwrap().1, "--nocapture"])
            .env(CHILD, temp.path())
            .output()
            .unwrap();
        let envelope = temp.path().join("stage/gate-envelope.json");
        assert!(
            output.status.success() && envelope.exists(),
            "the staged freeze ended {} with no envelope: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        );
        let value: serde_json::Value =
            serde_json::from_slice(&std::fs::read(envelope).unwrap()).unwrap();
        assert_eq!(value["operational_error"]["kind"], "authored_judge_reply");
        let text = value["operational_error"]["text"].as_str().unwrap();
        assert!(text.contains("malformed batched JSON"), "{text}");
        return;
    };
    let (project, _outside, tasks, prd) =
        crate::command::workflow_task_set::outside_set("test -f out");
    let contract = archon_workflow::task_set_contract::ACCEPTANCE_CONTRACT_FILE;
    let candidate = std::fs::read(tasks.join(contract)).unwrap();
    let staging = dir.join("stage");
    std::fs::create_dir_all(&staging).unwrap();
    let client = std::sync::Arc::new(
        crate::command::workflow_task_set::reauthor::test_client::ScriptedAuthorJudge::new(
            |entry, _| {
                crate::command::workflow_task_set::reauthor::test_client::command_entry(
                    entry,
                    "test -f out",
                )
            },
            |_, _| true,
        )
        .with_judge_reply("{ truncated"),
    );
    let resume = staged_freeze_resume(&mut std::io::sink());
    stage_acceptance_candidate(
        project.path(),
        (&tasks, &prd),
        &ArchonConfig::default(),
        client,
        &resume,
        StagedArgs {
            staging_root: &staging,
            gate_envelope: &staging.join("gate-envelope.json"),
            call_id: "call-1",
        },
        &candidate,
    )
    .await
    .unwrap();
}
