//! Issue 361: logic versions in the host-command reuse key, and the guard
//! that keeps them honest.
use std::collections::BTreeSet;

use archon_workflow::{HostCommandRequest, WorkflowV2CallRecord, host_command_call_id};

use crate::command::workflow_host_command_catalog::{
    fixed_decomposition_catalog, host_command_identity_tokens,
};
use crate::command::workflow_host_command_exec::{
    FixedHostCommandExecutor, WorkflowHostCommandExecutor,
};
use crate::command::workflow_host_command_logic::{
    BASELINE_LOGIC_VERSION, CAPABILITY_LOGIC, LOGIC_BUILD_STAMP, LOGIC_DIGEST_STAMP,
    LOGIC_VERSION_STAMP, THIS_BUILD, judges_only, outcome_logic_holds,
};

#[test]
fn logic_361_every_catalog_capability_has_a_logic_version() {
    let catalog = fixed_decomposition_catalog("any").unwrap();
    let catalog_ids = catalog
        .capabilities
        .keys()
        .cloned()
        .collect::<BTreeSet<_>>();
    let declared = CAPABILITY_LOGIC
        .iter()
        .map(|logic| logic.id.to_string())
        .collect::<BTreeSet<_>>();
    assert_eq!(declared.len(), CAPABILITY_LOGIC.len(), "one entry per id");
    assert_eq!(catalog_ids, declared);
    assert!(
        CAPABILITY_LOGIC
            .iter()
            .all(|logic| logic.version >= BASELINE_LOGIC_VERSION && !logic.sources.is_empty())
    );
    // The checks before Completed publish nothing but their verdict.
    let checks = catalog
        .capabilities
        .values()
        .filter(|capability| judges_only(capability))
        .map(|capability| capability.id.as_str())
        .collect::<BTreeSet<_>>();
    assert_eq!(
        checks,
        BTreeSet::from([
            "requirements-trace",
            "task-set-lint",
            "verify-frozen-acceptance",
            "verify-frozen-skeleton",
        ])
    );
}

fn record(command: &str, data: serde_json::Value) -> WorkflowV2CallRecord {
    let mut call = archon_workflow::WorkflowV2HostCall {
        id: format!("host-command:{command}"),
        method: archon_workflow::WorkflowV2HostMethod::HostCommand,
        write_mode: None,
        options: Default::default(),
    };
    call.options.host_command = Some(HostCommandRequest::new(command, None).unwrap());
    WorkflowV2CallRecord::new(
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
fn logic_361_policy_for_stamped_and_unstamped_outcomes() {
    let stamped = |version: serde_json::Value| serde_json::json!({ LOGIC_VERSION_STAMP: version });
    let unstamped = serde_json::json!({"exitCode": 0});
    // Stamped: only under the version it records.
    assert!(outcome_logic_holds(&stamped(1.into()), 1, true, None));
    assert!(outcome_logic_holds(&stamped(2.into()), 2, false, None));
    assert!(!outcome_logic_holds(&stamped(1.into()), 2, false, None));
    assert!(!outcome_logic_holds(&stamped(2.into()), 1, true, None));
    assert!(
        !outcome_logic_holds(&stamped("1".into()), 1, true, None),
        "malformed"
    );
    // Unstamped: a landing at the baseline answers, a check never does.
    assert!(outcome_logic_holds(
        &unstamped,
        BASELINE_LOGIC_VERSION,
        false,
        None
    ));
    assert!(!outcome_logic_holds(
        &unstamped,
        BASELINE_LOGIC_VERSION,
        true,
        None
    ));
    assert!(!outcome_logic_holds(
        &unstamped,
        BASELINE_LOGIC_VERSION + 1,
        false,
        None
    ));
}

/// The cheap checks replay only under the build that judged them; the
/// others keep version-only reuse (Steven decides on those).
#[test]
fn logic_361_cheap_checks_replay_only_under_the_same_build() {
    let stamped = |build: Option<serde_json::Value>| {
        let mut data = serde_json::json!({ LOGIC_VERSION_STAMP: 1, LOGIC_DIGEST_STAMP: "pin" });
        if let Some(build) = build {
            data[LOGIC_BUILD_STAMP] = build;
        }
        data
    };
    let bound = Some("build-b");
    // The same build: it replays.
    assert!(outcome_logic_holds(
        &stamped(Some("build-b".into())),
        1,
        true,
        bound
    ));
    // Another build, no build (an earlier binary), or a malformed one: never.
    assert!(!outcome_logic_holds(
        &stamped(Some("build-a".into())),
        1,
        true,
        bound
    ));
    assert!(!outcome_logic_holds(&stamped(None), 1, true, bound));
    assert!(!outcome_logic_holds(
        &stamped(Some(7.into())),
        1,
        true,
        bound
    ));
    // The same build never rescues another version.
    assert!(!outcome_logic_holds(
        &stamped(Some("build-b".into())),
        2,
        true,
        bound
    ));
    // Not bound: the version alone decides.
    assert!(outcome_logic_holds(
        &stamped(Some("build-a".into())),
        1,
        true,
        None
    ));
    assert!(outcome_logic_holds(&stamped(None), 1, false, None));
}

#[test]
fn logic_361_fixed_executor_binds_only_verify_and_trace_to_the_build() {
    let temp = tempfile::tempdir().unwrap();
    let context = crate::command::workflow_host_command_exec_tests::context(temp.path());
    let catalog = fixed_decomposition_catalog("rev-a").unwrap();
    let launched =
        FixedHostCommandExecutor::new(catalog.clone(), context.clone(), temp.path().join("run"));
    // Another binary, with the same versions and the same pinned digests.
    let rebuilt = FixedHostCommandExecutor::new(catalog, context, temp.path().join("run"))
        .with_build("other-build");
    for logic in CAPABILITY_LOGIC {
        let request = HostCommandRequest::new(logic.id, None).unwrap();
        // The host stamps the pinned digest and this binary's build.
        let digest = launched.logic_digest(&request).unwrap();
        assert_eq!(
            digest.as_deref(),
            Some(logic.sources_digest),
            "{}",
            logic.id
        );
        let build = launched.logic_build(&request).unwrap();
        assert_eq!(build.as_deref(), Some(THIS_BUILD), "{}", logic.id);
        let data = serde_json::json!({
            LOGIC_VERSION_STAMP: logic.version,
            LOGIC_DIGEST_STAMP: digest,
            LOGIC_BUILD_STAMP: build,
        });
        let holds = |executor: &FixedHostCommandExecutor| {
            executor
                .outcome_logic_holds(&record(logic.id, data.clone()))
                .unwrap()
        };
        assert!(holds(&launched), "{}: same build", logic.id);
        let cheap = [
            "verify-frozen-acceptance",
            "verify-frozen-skeleton",
            "requirements-trace",
        ];
        assert_eq!(
            holds(&rebuilt),
            !cheap.contains(&logic.id),
            "{}: other build",
            logic.id
        );
    }
}

#[test]
fn logic_361_fixed_executor_judges_records_by_their_logic() {
    let temp = tempfile::tempdir().unwrap();
    let context = crate::command::workflow_host_command_exec_tests::context(temp.path());
    let catalog = fixed_decomposition_catalog("rev-a").unwrap();
    let executor = |bumped: Option<&str>| {
        let executor = FixedHostCommandExecutor::new(
            catalog.clone(),
            context.clone(),
            temp.path().join("run"),
        );
        match bumped {
            Some(id) => executor.with_logic_version(id, Some(BASELINE_LOGIC_VERSION + 1)),
            None => executor,
        }
    };
    let unstamped = serde_json::json!({"exitCode": 0});
    let holds = |executor: &FixedHostCommandExecutor, command: &str, data: &serde_json::Value| {
        executor
            .outcome_logic_holds(&record(command, data.clone()))
            .unwrap()
    };
    let baseline = executor(None);
    for command in ["freeze-acceptance", "freeze-skeleton"] {
        assert!(
            holds(&baseline, command, &unstamped),
            "{command}: landed artifact"
        );
    }
    assert!(
        !holds(&baseline, "land-task-body", &unstamped),
        "the newly versioned body landing must not reuse an unstamped verdict"
    );
    for command in [
        "verify-frozen-acceptance",
        "verify-frozen-skeleton",
        "task-set-lint",
        "requirements-trace",
    ] {
        assert!(
            !holds(&baseline, command, &unstamped),
            "{command}: unversioned check"
        );
        let request = HostCommandRequest::new(command, None).unwrap();
        let stamped = serde_json::json!({
            LOGIC_VERSION_STAMP: baseline.logic_version(&request).unwrap(),
            LOGIC_DIGEST_STAMP: baseline.logic_digest(&request).unwrap(),
            LOGIC_BUILD_STAMP: baseline.logic_build(&request).unwrap(),
        });
        assert!(holds(&baseline, command, &stamped), "{command}: same logic");
    }
    let bumped = executor(Some("freeze-skeleton"));
    assert!(!holds(&bumped, "freeze-skeleton", &unstamped));
    assert!(
        holds(&bumped, "freeze-acceptance", &unstamped),
        "others keep"
    );
    // The stamp the host writes is the version the key names.
    let request = HostCommandRequest::new("freeze-skeleton", Some("c".into())).unwrap();
    assert_eq!(bumped.logic_version(&request).unwrap(), Some(2));
    assert_eq!(baseline.logic_version(&request).unwrap(), Some(1));
    // A command this build no longer declares has no logic to vouch for it.
    let mut narrowed = catalog.clone();
    narrowed.capabilities.remove("freeze-skeleton");
    narrowed.recompute_digest().unwrap();
    let narrowed = FixedHostCommandExecutor::new(narrowed, context, temp.path().join("run"));
    assert!(!holds(&narrowed, "freeze-skeleton", &unstamped));
}

#[test]
fn logic_361_keys_at_the_baseline_are_the_keys_records_already_hold() {
    let temp = tempfile::tempdir().unwrap();
    let context = crate::command::workflow_host_command_exec_tests::context(temp.path());
    let catalog = fixed_decomposition_catalog("rev-a").unwrap();
    let executor =
        FixedHostCommandExecutor::new(catalog.clone(), context.clone(), temp.path().join("run"));
    for (command, stdin) in [
        ("verify-frozen-acceptance", None),
        ("freeze-skeleton", Some("candidate")),
        ("requirements-trace", None),
    ] {
        let request = HostCommandRequest::new(command, stdin.map(str::to_string)).unwrap();
        // The key a binary without logic versions computed.
        let legacy = host_command_call_id(
            command,
            &catalog.digest,
            &catalog.starting_binary_revision,
            &host_command_identity_tokens(&context, command).unwrap(),
            stdin.unwrap_or_default().as_bytes(),
        );
        assert_eq!(
            executor.call_identity(&request).unwrap(),
            legacy,
            "{command}"
        );
    }
}

#[test]
fn logic_361_a_bump_rekeys_that_capability_alone_and_none_names_no_key() {
    let temp = tempfile::tempdir().unwrap();
    let context = crate::command::workflow_host_command_exec_tests::context(temp.path());
    let launch = fixed_decomposition_catalog("rev-a").unwrap();
    // A later binary: another revision, same capabilities, resumed on the run.
    let current = fixed_decomposition_catalog("rev-b").unwrap();
    let build = |logic: Option<(&str, Option<u32>)>| {
        let executor = FixedHostCommandExecutor::new(
            current.clone(),
            context.clone(),
            temp.path().join("run"),
        )
        .with_launch_catalog(launch.clone());
        match logic {
            Some((id, version)) => executor.with_logic_version(id, version),
            None => executor,
        }
    };
    let launched =
        FixedHostCommandExecutor::new(launch.clone(), context.clone(), temp.path().join("run"));
    let skeleton = HostCommandRequest::new("freeze-skeleton", Some("c".into())).unwrap();
    let verify = HostCommandRequest::new("verify-frozen-acceptance", None).unwrap();
    let same = build(None);
    for request in [&skeleton, &verify] {
        assert_eq!(
            same.call_identity(request).unwrap(),
            launched.call_identity(request).unwrap(),
            "an upgrade that changes no logic keeps {}'s key",
            request.command_id
        );
    }
    let bumped = build(Some(("freeze-skeleton", Some(2))));
    assert_ne!(
        bumped.call_identity(&skeleton).unwrap(),
        launched.call_identity(&skeleton).unwrap()
    );
    assert_eq!(
        bumped.call_identity(&verify).unwrap(),
        launched.call_identity(&verify).unwrap()
    );
    let again = build(Some(("freeze-skeleton", Some(3))));
    assert_ne!(
        again.call_identity(&skeleton).unwrap(),
        bumped.call_identity(&skeleton).unwrap()
    );
    let undeclared = build(Some(("freeze-skeleton", None)));
    let error = undeclared.call_identity(&skeleton).unwrap_err().to_string();
    assert!(error.contains("has no logic version"), "{error}");
}

/// The cheap checks bind to the binary, not to a pinned digest: a record
/// that carries this build's version and pinned digest but no build stamp
/// (a binary that stamped none, or a stale pin shipped by a build that
/// skipped the guard test) never replays one.
#[test]
fn logic_361_a_cheap_check_without_this_builds_stamp_never_replays() {
    let temp = tempfile::tempdir().unwrap();
    let context = crate::command::workflow_host_command_exec_tests::context(temp.path());
    let catalog = fixed_decomposition_catalog("rev-a").unwrap();
    let executor = FixedHostCommandExecutor::new(catalog, context, temp.path().join("run"));
    for logic in CAPABILITY_LOGIC {
        let data = serde_json::json!({
            LOGIC_VERSION_STAMP: logic.version,
            LOGIC_DIGEST_STAMP: logic.sources_digest,
        });
        let holds = executor
            .outcome_logic_holds(&record(logic.id, data))
            .unwrap();
        assert_eq!(holds, !logic.build_bound, "{}", logic.id);
    }
}
