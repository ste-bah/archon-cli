//! Issue 361: logic versions in the host-command reuse key, and the guard
//! that keeps them honest.
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use archon_workflow::{HostCommandRequest, WorkflowV2CallRecord, host_command_call_id};

use crate::command::workflow_host_command_catalog::{
    fixed_decomposition_catalog, host_command_identity_tokens,
};
use crate::command::workflow_host_command_exec::{
    FixedHostCommandExecutor, WorkflowHostCommandExecutor,
};
use crate::command::workflow_host_command_logic::{
    BASELINE_LOGIC_VERSION, CAPABILITY_LOGIC, LOGIC_VERSION_STAMP, judges_only, outcome_logic_holds,
};

/// Test modules never stand for a capability's logic.
fn is_test_file(file: &Path) -> bool {
    let stem = file.file_stem().and_then(|s| s.to_str()).unwrap_or("");
    stem == "tests"
        || stem.ends_with("_tests")
        || stem.contains("_tests_")
        || stem.ends_with("_test")
        || stem.contains("test_support")
        || stem.contains("fixture")
}

/// The module `line` declares out of line (`mod name;`), if any.
fn declared_module(line: &str) -> Option<&str> {
    let line = line.trim_start();
    let line = match line.strip_prefix("pub") {
        Some(rest) if rest.starts_with('(') => rest.split_once(')')?.1.trim_start(),
        Some(rest) => rest.trim_start(),
        None => line,
    };
    let name = line.strip_prefix("mod ")?.trim().strip_suffix(';')?.trim();
    name.chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_')
        .then_some(name)
}

/// `file` and every out-of-line module it declares, recursively, except the
/// ones only tests compile: a new submodule under a root is covered unasked.
fn subtree(file: &Path, out: &mut BTreeSet<PathBuf>) {
    if is_test_file(file) || out.contains(file) {
        return;
    }
    let text = std::fs::read_to_string(file)
        .unwrap_or_else(|error| panic!("logic source {} unreadable: {error}", file.display()));
    out.insert(file.to_path_buf());
    let (mut path_attr, mut test_only) = (None::<String>, false);
    for raw in text.lines() {
        let mut line = raw.trim();
        while let Some(rest) = line.strip_prefix("#[") {
            let Some((attr, after)) = rest.split_once(']') else {
                break;
            };
            if let Some(path) = attr.strip_prefix("path = \"") {
                path_attr = path.split('"').next().map(str::to_string);
            }
            if attr.starts_with("cfg(") && attr.contains("test") {
                test_only = true;
            }
            line = after.trim();
        }
        if line.is_empty() || line.starts_with("//") {
            continue;
        }
        if let Some(name) = declared_module(line)
            && !test_only
        {
            let dir = file.parent().expect("a source file has a directory");
            let child = match &path_attr {
                Some(path) => dir.join(path),
                None => {
                    let stem = file.file_stem().and_then(|s| s.to_str()).unwrap_or("");
                    let base = if matches!(stem, "mod" | "lib" | "main") {
                        dir.to_path_buf()
                    } else {
                        dir.join(stem)
                    };
                    let flat = base.join(format!("{name}.rs"));
                    if flat.exists() {
                        flat
                    } else {
                        base.join(name).join("mod.rs")
                    }
                }
            };
            subtree(&child, out);
        }
        path_attr = None;
        test_only = false;
    }
}

/// The digest of the module subtrees of `roots`, each file named by its path
/// under `base` and read with LF line ends, so a checkout's line-end setting
/// never moves it.
pub(crate) fn sources_digest(base: &Path, roots: &[&str]) -> (String, usize) {
    let mut files = BTreeSet::new();
    for root in roots {
        subtree(&base.join(root), &mut files);
    }
    let mut framed = Vec::new();
    for file in &files {
        let name = file
            .strip_prefix(base)
            .expect("logic sources live in the repository")
            .components()
            .map(|part| part.as_os_str().to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join("/");
        let text = std::fs::read_to_string(file).unwrap().replace("\r\n", "\n");
        for part in [name.as_bytes(), text.as_bytes()] {
            framed.extend_from_slice(&(part.len() as u64).to_le_bytes());
            framed.extend_from_slice(part);
        }
    }
    (
        archon_workflow::task_set_contract::content_digest(&framed),
        files.len(),
    )
}

#[test]
fn logic_361_sources_match_their_pinned_digest() {
    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let stale = CAPABILITY_LOGIC
        .iter()
        .filter_map(|logic| {
            let roots = logic.sources.concat();
            let (digest, files) = sources_digest(&repo, &roots);
            (digest != logic.sources_digest).then(|| {
                format!(
                    "  {} (logic version {}, {files} files): sources digest is {digest}, pinned {}",
                    logic.id, logic.version, logic.sources_digest
                )
            })
        })
        .collect::<Vec<_>>();
    assert!(
        stale.is_empty(),
        "The source of these host command capabilities changed (Issue 361):\n{}\n\
         In src/command/workflow_host_command_logic.rs, for each one: if the change can \
         alter any verdict the subcommand gives, bump its `version` (its next call runs \
         again; every other capability keeps its results); if it cannot, keep the version. \
         Either way set `sources_digest` to the digest above.",
        stale.join("\n")
    );
}

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

/// The guard itself: a logic edit anywhere in a root's module subtree moves
/// the digest, a new submodule is covered without being listed, and a test
/// module never moves it.
#[test]
fn logic_361_the_guard_sees_every_logic_file_and_no_test() {
    let temp = tempfile::tempdir().unwrap();
    let base = temp.path();
    let write = |path: &str, text: &str| {
        let path = base.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    };
    write(
        "src/root.rs",
        "#[path = \"root_helper.rs\"]\nmod helper;\n#[cfg(test)]\nmod tests;\n#[cfg(test)] #[path = \"root_more_tests.rs\"] mod more;\npub(crate) mod inner;\n",
    );
    write("src/root_helper.rs", "pub fn verdict() -> bool { true }\n");
    write("src/root/inner.rs", "pub fn rule() {}\n");
    write("src/root/tests.rs", "#[test] fn t() {}\n");
    write("src/root_more_tests.rs", "#[test] fn u() {}\n");
    let digest = || sources_digest(base, &["src/root.rs"]);
    let (first, files) = digest();
    assert_eq!(files, 3, "root, helper and inner; no test module");
    write("src/root/tests.rs", "#[test] fn t() { assert!(true) }\n");
    write(
        "src/root_more_tests.rs",
        "#[test] fn u() { assert!(true) }\n",
    );
    assert_eq!(digest().0, first, "a test edit is not a logic change");
    write("src/root_helper.rs", "pub fn verdict() -> bool { false }\n");
    let second = digest().0;
    assert_ne!(second, first, "a helper edit is a logic change");
    write("src/root/inner.rs", "mod deeper;\npub fn rule() {}\n");
    write("src/root/inner/deeper.rs", "pub fn stricter() {}\n");
    let (third, files) = digest();
    assert_eq!(files, 4, "a new submodule is covered unlisted");
    assert_ne!(third, second);
    write("src/root/inner/deeper.rs", "pub fn stricter() {}\r\n");
    assert_eq!(digest().0, third, "line ends are not logic");
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
    assert!(outcome_logic_holds(&stamped(1.into()), 1, true));
    assert!(outcome_logic_holds(&stamped(2.into()), 2, false));
    assert!(!outcome_logic_holds(&stamped(1.into()), 2, false));
    assert!(!outcome_logic_holds(&stamped(2.into()), 1, true));
    assert!(
        !outcome_logic_holds(&stamped("1".into()), 1, true),
        "malformed"
    );
    // Unstamped: a landing at the baseline answers, a check never does.
    assert!(outcome_logic_holds(
        &unstamped,
        BASELINE_LOGIC_VERSION,
        false
    ));
    assert!(!outcome_logic_holds(
        &unstamped,
        BASELINE_LOGIC_VERSION,
        true
    ));
    assert!(!outcome_logic_holds(
        &unstamped,
        BASELINE_LOGIC_VERSION + 1,
        false
    ));
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
    for command in ["freeze-acceptance", "freeze-skeleton", "land-task-body"] {
        assert!(
            holds(&baseline, command, &unstamped),
            "{command}: landed artifact"
        );
    }
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
        let stamped = serde_json::json!({ LOGIC_VERSION_STAMP: BASELINE_LOGIC_VERSION });
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
        ("task-set-lint", None),
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
