use super::*;

pub async fn run_at(
    site: &CommandSite<'_>,
    id: &str,
    command: &crate::acceptance_world::AuthorizedCommand,
    cancel: Arc<AtomicBool>,
) -> WorkflowResult<CheckResult> {
    let cwd = match command.cwd() {
        crate::task_set_contract::TrustedCwd::ProjectRoot => site.project,
        crate::task_set_contract::TrustedCwd::RepoRoot => site.repository,
    };
    let mut environment = site.environment.clone();
    let mut values: Vec<_> = environment.into_iter().collect();
    archon_tools::workflow_resource_env::apply_workflow_resource_defaults(
        &mut values,
        &String::from_utf8_lossy(command.bytes()),
        site.repository,
        &site.resource_limits,
    );
    environment = values.into_iter().collect();
    let mut process = archon_shell::spawn::tokio_command(archon_shell::resolve_posix_shell());
    archon_shell::spawn::replace_environment(process.as_std_mut(), environment);
    process
        .arg("-s")
        .current_dir(cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    // A scratch slot lists every group it spawns, so a later holder can
    // tell whether one outlived a killed observation (`cache`).
    if let Some(root) = site.audit_root {
        super::cache::record_group(root, None)?;
    }
    let moved = site
        .mutation_plan
        .map(|plan| mutation::move_aside_plan(plan, site.mutation_root.as_ref()));
    let parent_snapshots = site.mutation_plan.zip(moved.as_ref()).map(|(plan, moved)| {
        mutation::snapshot_parent_states(plan, moved, site.mutation_root.as_ref())
    });
    #[cfg(not(windows))]
    let child = spawn_confined(process, cwd)?;
    #[cfg(windows)]
    let mut owned = spawn_confined(process, cwd)?;
    #[cfg(windows)]
    let child = &mut owned.child;
    let leader = child
        .id()
        .ok_or_else(|| invalid("scratch child has no process id"))?;
    #[cfg(unix)]
    let mut owned = confine::OwnedCheck::new(leader, child);
    #[cfg(unix)]
    let (child, confinement) = (&mut *owned.child, &mut owned.confinement);
    // Borrowed on every platform, as the Unix owner hands them out.
    #[cfg(not(any(unix, windows)))]
    let mut confinement = Confinement::new(leader);
    #[cfg(not(any(unix, windows)))]
    let confinement = &mut confinement;
    #[cfg(windows)]
    let (child, confinement) = (&mut owned.child, &mut owned.confinement);
    #[cfg(not(any(unix, windows)))]
    let mut child = child;
    #[cfg(not(any(unix, windows)))]
    let child = &mut child;
    if let Some(root) = site.audit_root {
        super::cache::record_group(root, Some(leader as i32))?;
    }
    let (stdout_pipe, stderr_pipe, stdin_pipe) = take_pipes(child);
    let progress = confinement.progress();
    progress.set_report_interval(
        (Duration::from_secs(site.timeout_secs) / 4)
            .clamp(Duration::from_millis(25), Duration::from_secs(60)),
    );
    let overflow = Arc::new(AtomicBool::new(false));
    let mut stdout = tokio::spawn(drain(
        stdout_pipe,
        site.output_bytes,
        overflow.clone(),
        progress.clone(),
    ));
    let mut stderr = tokio::spawn(drain(
        stderr_pipe,
        site.output_bytes,
        overflow.clone(),
        progress.clone(),
    ));
    let mut stdin = stdin_pipe;
    let bytes = command.bytes().to_vec();
    let writer = tokio::spawn(async move {
        stdin.write_all(&bytes).await?;
        stdin.write_all(b"\n").await?;
        stdin.shutdown().await
    });
    // A limit past the clock's range is no deadline at all (Issue 263),
    // never an overflow that panics the host.
    let window = Duration::from_secs(site.timeout_secs);
    let mut error = None;
    let mut fault = None;
    let mut quota_walk_count = 0;
    // A site with no audit root never walks a quota; the branch below stays
    // dormant by never being scheduled rather than by a flag it could forget.
    let quota_period = if site.audit_root.is_some() {
        Duration::from_secs(5)
    } else {
        Duration::from_secs(365 * 86_400)
    };
    let mut next_quota = tokio::time::Instant::now() + quota_period;
    let mut stall = None;
    let mut ticks = 0u32;
    // `None`: the leader exited and is not reaped yet (Unix); `Some`: it was
    // stopped and reaped, with its status if that is known.
    let stopped = loop {
        tokio::select! {
            result=leader_exit(child)=>break match result {
                Ok(()) => None,
                Err(e) => { stall = Some(format!("waiting for the scratch child failed: {e}")); None }
            },

            _=tokio::time::sleep(Duration::from_millis(25))=>{
                if cancel.load(Ordering::SeqCst) {error=Some("observation parent closed or cancellation requested".into());break Some(terminate(child,confinement,&mut stall).await);}
                if overflow.load(Ordering::SeqCst) {error=Some(super::faults::OUTPUT_LIMIT_EXCEEDED.into()); fault=Some(super::faults::FaultCause::OutputLimitExceeded); break Some(terminate(child,confinement,&mut stall).await);}
                if progress.stalled(window) {error=Some(CHECK_TIMED_OUT.into());break Some(terminate(child,confinement,&mut stall).await);}
                ticks += 1;
                if ticks.is_multiple_of(SCAN_TICKS) { confinement.scan(); }
            }
            _=tokio::time::sleep_until(next_quota)=>{
                quota_walk_count += 1;
                if let Some(root) = site.audit_root {
                    match site.audited_size(root) {
                        Ok(size) if size>site.scratch_bytes=>{error=Some(super::faults::SCRATCH_LIMIT_EXCEEDED.into()); fault=Some(super::faults::FaultCause::ScratchLimitExceeded); break Some(terminate(child,confinement,&mut stall).await);}
                        Err(e)=>{error=Some(format!("scratch size audit failed: {e}")); fault=None; break Some(terminate(child,confinement,&mut stall).await);}
                        _=>{}
                    }
                }
                next_quota = tokio::time::Instant::now() + quota_period;
            }
        }
    };
    // A quiet, short check may finish before the first CPU sample. Its
    // observed natural exit is real activity across a batch of such checks.
    if stopped.is_none() && stall.is_none() {
        progress.record();
    }
    writer.abort();
    // Reap remaining members even if the leader exited successfully: the
    // whole tree, including every member a scan saw while the check ran,
    // while an exited leader is still unreaped; then reap it.
    let teardown = confinement.kill().await;
    let status = match stopped {
        Some(status) => status,
        None => reap(child, &mut stall).await,
    };
    confinement.leader_reaped();
    let mut pipes = match tokio::time::timeout(Duration::from_secs(3), async {
        let out = (&mut stdout)
            .await
            .map_err(|e| invalid(e.to_string()))?
            .map_err(|e| WorkflowError::io(cwd, e))?;
        let err = (&mut stderr)
            .await
            .map_err(|e| invalid(e.to_string()))?
            .map_err(|e| WorkflowError::io(cwd, e))?;
        Ok::<_, WorkflowError>((out, err))
    })
    .await
    {
        Ok(pipes) => pipes?,
        Err(_) => {
            stdout.abort();
            stderr.abort();
            error = Some(super::faults::PIPES_HELD.into());
            fault = Some(super::faults::FaultCause::PipesHeld);
            ((Vec::new(), false), (Vec::new(), false))
        }
    };
    // The tree is disarmed only once no member is left, never merely because
    // the leader exited.
    let process_tree_gone = match teardown {
        Ok(()) => {
            confinement.disarm();
            true
        }
        Err(evidence) => {
            stall = Some(evidence);
            false
        }
    };
    // A scratch is private to the observation: what still holds it after
    // the tree is gone may be a descendant nothing else names any more.
    #[cfg(unix)]
    if let Some(root) = site.audit_root
        && let Some(audit) = confinement.detached_holders(root, site.audit_target).await
    {
        record_holder_audit(audit, &mut error, &mut fault);
    }
    if overflow.load(Ordering::SeqCst) {
        error = Some(super::faults::OUTPUT_LIMIT_EXCEEDED.into());
        fault = Some(super::faults::FaultCause::OutputLimitExceeded);
    }
    if let Some(root) = site.audit_root {
        quota_walk_count += 1;
        match site.audited_size(root) {
            Ok(size) if size > site.scratch_bytes => {
                error = Some(super::faults::SCRATCH_LIMIT_EXCEEDED.into());
                fault = Some(super::faults::FaultCause::ScratchLimitExceeded);
            }
            Err(e) => {
                error = Some(format!("scratch size audit failed: {e}"));
                fault = None;
            }
            _ => {}
        }
    }
    // A stall outranks every other ending: the check could not be torn down,
    // and that is what a resume has to know.
    if let Some(evidence) = stall {
        fault = finish::teardown_fault(&evidence);
        error = Some(evidence);
    }
    let result = CheckResult {
        verdict_streams: Default::default(),
        fault,
        classification: None,
        acceptance_id: id.into(),
        exit_code: status.and_then(|status| status.code()),
        quota_walk_count,
        stdout: std::mem::take(&mut pipes.0.0),
        stderr: std::mem::take(&mut pipes.1.0),
        operational_error: error,
        environment_note: None,
    };
    finish::finish(
        result,
        site,
        command,
        &pipes,
        process_tree_gone,
        moved.as_deref(),
        parent_snapshots.as_deref(),
    )
}
