//! Issue 358: limits are not in a host command's reuse key, but an outcome a
//! limit cut short replays only under the limits it ran under (its own stamp);
//! a catalog schema this build cannot read names no key at all.
use super::*;

/// A cut-short freeze, first run by the launch build (`ran_under` "launch")
/// or by an upgrade, then resumed by each of `phases` (an upgrade, and how
/// many calls it must run): it replays while the limits it ran under hold
/// and runs again once they change. The landed freeze after it is reused.
async fn cut_short_freeze(stdin: &str, ran_under: &str, phases: &[(&str, usize)]) {
    let temp = tempfile::tempdir().unwrap();
    let spec = test_spec();
    let store = WorkflowStore::new(temp.path().join("workflows"));
    let run = store.create_run(spec.clone()).unwrap();
    let v2 = WorkflowV2ResultStore::new(store.run_dir(&run.id).join("v2"));
    let context = crate::command::workflow_host_command_exec_tests::context(temp.path());
    let launch = launch_catalog();
    let first = Arc::new(CatalogHost {
        keys: FixedHostCommandExecutor::new(
            launch.clone(),
            context.clone(),
            store.run_dir(&run.id),
        ),
        calls: AtomicUsize::new(0),
    });
    let upgraded = |change: &str| {
        Arc::new(CatalogHost {
            keys: FixedHostCommandExecutor::new(
                changed(&launch, "freeze-skeleton", change),
                context.clone(),
                store.run_dir(&run.id),
            )
            .with_launch_catalog(launch.clone()),
            calls: AtomicUsize::new(0),
        })
    };
    let (sink, _rx) = default_workflow_ui_sink();
    let runner = |executor: Arc<CatalogHost>| {
        WorkflowV2ScriptRunner::new(
            "cut short upgrade".into(),
            test_runtime(&spec),
            WorkflowV2AgentAdapter::new(),
            LiveV2AgentClient::new(
                Arc::new(PanicLlm),
                sink.clone(),
                Vec::new(),
                run.id.clone(),
                None,
                None,
            ),
            v2.clone(),
            store.clone(),
            run.id.clone(),
            true,
            None,
            None,
        )
        .with_raw_outcomes(true)
        .with_host_command_executor(executor)
    };
    // The cut-short freeze is history: a later landing supersedes it.
    let script = format!(
        r#"async function workflow(w) {{
        await w.hostCommand("freeze-skeleton", {{stdin: "{stdin}"}});
        await w.hostCommand("freeze-skeleton", {{stdin: "fast"}});
    }}"#
    );
    let ran = if ran_under == "launch" {
        first
    } else {
        upgraded(ran_under)
    };
    assert_eq!(runner(ran).run(&script).await.unwrap().executed, 2);
    for &(change, rerun) in phases {
        let second = upgraded(change);
        let summary = runner(second.clone()).run(&script).await.unwrap();
        assert_eq!(summary.executed, rerun, "{stdin}/{change}");
        assert_eq!(summary.reused, 2 - rerun, "{stdin}/{change}");
        assert_eq!(second.calls.load(Ordering::SeqCst), rerun);
    }
}

#[tokio::test]
async fn upgrade_358_timed_out_freeze_reruns_after_a_timeout_change() {
    cut_short_freeze("slow", "launch", &[("none", 0), ("timeout", 1)]).await;
}
#[tokio::test]
async fn upgrade_358_truncated_freeze_reruns_after_an_output_limit_change() {
    cut_short_freeze("loud", "launch", &[("none", 0), ("stdout_limit", 1)]).await;
}
#[tokio::test]
async fn upgrade_358_timed_out_freeze_reruns_after_a_limit_schema_bump() {
    cut_short_freeze("slow", "launch", &[("none", 0), ("schema", 1)]).await;
}

/// N1: cut short under the upgrade's own limits and superseded, then
/// resumed again on that build: history, replayed. A later limit change runs
/// it again; the stamp, not the launch catalog, says what cut it short.
#[tokio::test]
async fn upgrade_358_cut_short_under_the_upgrade_replays_under_it() {
    cut_short_freeze("slow", "timeout", &[("timeout", 0), ("timeout_again", 1)]).await;
}
#[tokio::test]
async fn upgrade_358_cut_short_under_a_schema_bump_replays_under_it() {
    cut_short_freeze("loud", "schema", &[("schema", 0), ("none", 1)]).await;
}

fn host_record(command: &str, data: serde_json::Value) -> WorkflowV2CallRecord {
    let mut call = archon_workflow::WorkflowV2HostCall {
        id: format!("host-command:{command}"),
        method: archon_workflow::WorkflowV2HostMethod::HostCommand,
        write_mode: None,
        options: Default::default(),
    };
    call.options.host_command =
        Some(archon_workflow::HostCommandRequest::new(command, None).unwrap());
    archon_workflow::WorkflowV2CallRecord::new(
        "run",
        call,
        1,
        "hash".into(),
        archon_workflow::WorkflowV2Result {
            data,
            ..Default::default()
        },
        Vec::new(),
    )
}

#[test]
fn upgrade_358_limits_hold_only_for_outcomes_no_limit_cut_short() {
    let temp = tempfile::tempdir().unwrap();
    let context = crate::command::workflow_host_command_exec_tests::context(temp.path());
    let launch = launch_catalog();
    let upgraded = FixedHostCommandExecutor::new(
        changed(&launch, "freeze-skeleton", "timeout"),
        context.clone(),
        temp.path().join("run"),
    )
    .with_launch_catalog(launch.clone());
    let stopped = serde_json::json!({"interrupted": "paused by operator"});
    let completed = serde_json::json!({"exitCode": 1, "timedOut": false, "interrupted": false});
    // A limit cut it short and the limit changed: not an answer now.
    assert!(
        !upgraded
            .outcome_limits_hold(&host_record("freeze-skeleton", stopped.clone()))
            .unwrap()
    );
    // No limit cut it short: the limit is not its input.
    assert!(
        upgraded
            .outcome_limits_hold(&host_record("freeze-skeleton", completed))
            .unwrap()
    );
    // Another capability's limits are unchanged.
    assert!(
        upgraded
            .outcome_limits_hold(&host_record("verify-frozen-acceptance", stopped.clone()))
            .unwrap()
    );
    // A stamped record is judged by its own stamp, not the launch limits.
    let stamp = upgraded
        .limits_fingerprint(
            &archon_workflow::HostCommandRequest::new("freeze-skeleton", None).unwrap(),
        )
        .unwrap()
        .expect("a declared capability has limits");
    let mut stamped = stopped.clone();
    stamped["limitsFingerprint"] = stamp.clone();
    assert!(
        upgraded
            .outcome_limits_hold(&host_record("freeze-skeleton", stamped.clone()))
            .unwrap()
    );
    let mut launch_stamped = stopped.clone();
    launch_stamped["limitsFingerprint"] =
        FixedHostCommandExecutor::new(launch.clone(), context.clone(), temp.path().join("run"))
            .limits_fingerprint(
                &archon_workflow::HostCommandRequest::new("freeze-skeleton", None).unwrap(),
            )
            .unwrap()
            .unwrap();
    assert!(
        !upgraded
            .outcome_limits_hold(&host_record("freeze-skeleton", launch_stamped))
            .unwrap()
    );
    // A command this build no longer declares has no limits to hold.
    let mut removed = launch.clone();
    removed.capabilities.remove("freeze-skeleton");
    removed.recompute_digest().unwrap();
    let narrowed = FixedHostCommandExecutor::new(removed, context, temp.path().join("run"))
        .with_launch_catalog(launch);
    assert!(
        !narrowed
            .outcome_limits_hold(&host_record("freeze-skeleton", stopped))
            .unwrap()
    );
}

#[test]
fn upgrade_358_unreadable_launch_catalog_schema_names_no_key() {
    let temp = tempfile::tempdir().unwrap();
    let context = crate::command::workflow_host_command_exec_tests::context(temp.path());
    let current = launch_catalog();
    let request =
        archon_workflow::HostCommandRequest::new("verify-frozen-acceptance", None).unwrap();
    for (schema, readable) in [
        (current.schema_version, true),
        (0, false),
        (current.schema_version + 1, false),
    ] {
        let mut launch = current.clone();
        launch.schema_version = schema;
        launch.recompute_digest().unwrap();
        let executor = FixedHostCommandExecutor::new(
            current.clone(),
            context.clone(),
            temp.path().join("run"),
        )
        .with_launch_catalog(launch);
        match executor.call_identity(&request) {
            Ok(_) => assert!(readable, "schema {schema} must not name a key"),
            Err(error) => assert!(
                !readable && error.to_string().contains(&format!("schema {schema}")),
                "schema {schema}: {error}"
            ),
        }
    }
}

/// N3: a catalog schema bump cannot pass until someone decides what the new
/// schema means for reuse and lists it.
#[test]
fn upgrade_358_every_built_catalog_schema_is_known() {
    use crate::command::workflow_host_command_exec::identity::{
        KNOWN_CATALOG_SCHEMAS, catalog_schema_readable,
    };
    let current = crate::command::workflow_host_command_catalog::fixed_decomposition_catalog("any")
        .unwrap()
        .schema_version;
    for schema in 1..=current {
        assert!(
            KNOWN_CATALOG_SCHEMAS
                .iter()
                .any(|(known, _)| *known == schema),
            "catalog schema {schema} is built but not in KNOWN_CATALOG_SCHEMAS: decide its reuse meaning"
        );
        assert!(catalog_schema_readable(schema, current));
    }
    // Unknown, and newer than this build: refused.
    assert!(!catalog_schema_readable(0, current));
    assert!(!catalog_schema_readable(current + 1, current));
}
