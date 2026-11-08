use super::*;

/// Execute the declared command under the host write boundary when a run
/// context is available.
pub(super) async fn run_contract_verifier_for(
    command: &str,
    timeout: std::time::Duration,
    run_root: Option<&std::path::Path>,
    require_status: bool,
    classifier: Option<TypedProgramClassifier>,
) -> ContractVerification {
    // Feed command text on stdin to avoid argument-length limits.
    // Issue-227/234: `boundary` lives until the verifier has been reaped, then
    // `finish` restores and names any change to a sealed root on a host with no
    // kernel boundary (a no-op where the kernel refused the write live).
    let (mut process, boundary, environment) = match crate::write_coordinator::host_sandbox::command(
        archon_shell::resolve_posix_shell(),
        run_root,
        &[],
    ) {
        Ok((command, boundary, environment)) => (
            tokio::process::Command::from(command),
            boundary,
            environment,
        ),
        Err(reason) => return ContractVerification::Unavailable(reason),
    };
    process
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    // Batch G2: its own process group, so a timeout reaps what it started.
    #[cfg(unix)]
    process.process_group(0);
    let variables = process
        .as_std()
        .get_envs()
        .filter_map(|(key, value)| Some((key.to_str()?.to_string(), value?.to_str()?.to_string())))
        .collect();
    let mut child = match archon_shell::read_boundary::prepare_spawn(process.as_std())
        .and_then(|()| process.spawn())
    {
        Ok(child) => child,
        Err(error) => {
            return ContractVerification::Unavailable(format!(
                "host could not execute the declared contract verifier: {error}"
            ));
        }
    };
    let pid = child.id();
    // Issue-134: dropped mid-run, the whole group goes with it.
    let _group = crate::v2::write::test_baseline_run::GroupKillOnDrop(pid);
    if let Some(mut stdin) = child.stdin.take() {
        use tokio::io::AsyncWriteExt as _;
        let script = command.to_string();
        // Ignore write failures: the child may have exited already, and the
        // output/exit status below is what decides the verdict either way.
        let _ = stdin.write_all(script.as_bytes()).await;
        let _ = stdin.shutdown().await;
    }
    let run = bounded_output(child);
    let output = match tokio::time::timeout(timeout, run).await {
        Ok(Ok(output)) => output,
        // Fail closed: an unrunnable verifier is not evidence of success --
        // and not evidence of failure either (Batch G2).
        Ok(Err(error)) => {
            crate::v2::write::test_baseline_run::kill_group(pid);
            return ContractVerification::Unavailable(format!(
                "host could not execute the declared contract verifier: {error}"
            ));
        }
        Err(_) => {
            crate::v2::write::test_baseline_run::kill_group(pid);
            return ContractVerification::Unavailable(format!(
                "declared contract verifier did not finish within {}s; treating as unverified",
                timeout.as_secs()
            ));
        }
    };
    // Reap anything the verifier left running behind it.
    crate::v2::write::test_baseline_run::kill_group(pid);
    // Issue-234: on a host with no kernel boundary, restore any sealed root the
    // verifier changed. A change means the tree it was judged against is not the
    // one the host sealed, so there is no trusted verdict: re-run it.
    if let Err(reason) = boundary.finish("the declared contract verifier for this run") {
        return ContractVerification::Unavailable(reason);
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let verdicts = verifier_verdicts(&stdout);
    // Issue 219: past the read cap the verdict cannot be read whole. What
    // was read still counts; a pass never does.
    let cut = (output.stdout_total > output.stdout.len() as u64).then(|| {
        format!(
            "declared contract verifier output cut at {} of {} bytes [output cut]: any failure printed after the cut is unread, and its verdict is not trusted as a pass",
            output.stdout.len(),
            output.stdout_total
        )
    });
    // Any stage that reported a failure demotes the branch, whichever one it
    // was. Returning on the FIRST verdict instead would let the typed
    // pre-check's permissive `{"status":"verified"}` mask the contract
    // verifier's own failure printed after it.
    let failures: Vec<String> = verdicts
        .iter()
        .filter_map(verdict_failure)
        .flatten()
        .collect();
    // Notes accompany every real failure, including JSON verdict failures on
    // exit zero. They never turn a branch failure into Unavailable (Issue 349).
    let failed = |findings: Vec<String>| {
        ContractVerification::Failed(
            findings,
            environment.note(&[&output.stdout, &output.stderr]),
        )
    };
    let classification = crate::acceptance_check_crash::classify_execution(
        command,
        output.status.code(),
        &String::from_utf8_lossy(&output.stderr),
        &[],
    );
    if !failures.is_empty() || cut.is_some() {
        return failed(failures.into_iter().chain(cut).collect());
    }
    if !require_status
        && matches!(output.status.code(), Some(126 | 127))
        && let Some(classifier) = classifier
    {
        let result = crate::acceptance_scratch::CheckResult {
            acceptance_id: String::new(),
            exit_code: output.status.code(),
            quota_walk_count: 0,
            stdout: output.stdout.clone(),
            stderr: output.stderr.clone(),
            environment_note: None,
            operational_error: None,
            fault: None,
            classification: None,
            verdict_streams: Default::default(),
        };
        if let Some(reason) = classifier(command, &result, &variables) {
            return ContractVerification::Unverified(format!(
                "unverified: typed command unavailable ({reason})"
            ));
        }
    }
    if !matches!(
        classification,
        crate::acceptance_check_crash::CheckRunClass::Passed
    ) {
        // Its end and its failure lines (stdout when stderr is silent),
        // well within one finding's 4 KB gap bound (`contracts_demote`).
        let said = if output.stderr.trim_ascii().is_empty() {
            &output.stdout
        } else {
            &output.stderr
        };
        // A non-zero exit and a shell expansion abort that exited 0 are
        // different facts; name the one that happened.
        let what = if output.status.success() {
            "aborted on a shell expansion error"
        } else {
            "exited non-zero"
        };
        return failed(vec![format!(
            "declared contract verifier {what}: {}",
            crate::failure_evidence::failure_evidence(said, 420)
        )]);
    }
    // Standalone command callers require a status; a typed command's exit
    // classification suffices after its predicates passed.
    if !require_status
        || verdicts.iter().rev().any(|verdict| {
            verdict
                .get("status")
                .and_then(serde_json::Value::as_str)
                .is_some()
        })
    {
        return ContractVerification::Passed;
    }
    failed(vec![
        "declared contract verifier produced no parseable status; treating as unverified"
            .to_string(),
    ])
}
