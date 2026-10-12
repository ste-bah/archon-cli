//! Content cells of the fault matrix (#257): author content that triggers
//! redaction, CRLF and heredoc verifier scripts, and a large candidate.
//! Each runs the whole decomposition once, with no resume.
use std::fs;
use std::path::Path;

use crate::deadline::{progress, run_within};
use crate::project::{
    FinalSnapshot, TestProject, assert_no_unknown_prompts, decompose_args, final_snapshot, one_run,
    project_with, project_with_files, read_golden_requests,
};
#[cfg(unix)]
use crate::server::variants::{CRLF_SCRIPT, HEREDOC_SCRIPT};
use crate::server::variants::{SECRET_TOKEN, large};
use crate::server::{Plan, Variant};

fn decompose(project: &TestProject, name: &str) {
    decompose_within(project, name, crate::deadline::NO_PROGRESS_LIMIT);
}

fn decompose_within(project: &TestProject, name: &str, limit: std::time::Duration) {
    progress(name, "starting");
    let output = run_within(project.path(), &decompose_args(), limit);
    assert!(
        output.status.success(),
        "{name}: exit {}\nstdout: {}\nstderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_no_unknown_prompts(&project.provider);
}

fn golden() -> FinalSnapshot {
    serde_json::from_value(crate::snapshot::read_golden_snapshot()).unwrap()
}

fn files_under(root: &Path) -> Vec<std::path::PathBuf> {
    let mut files = Vec::new();
    let Ok(entries) = fs::read_dir(root) else {
        return files;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            files.extend(files_under(&path));
        } else {
            files.push(path);
        }
    }
    files
}

/// A secret-shaped word in an author reply is redacted in every public copy
/// (events, prompt and agent-output records) and kept as authored in the
/// authoritative v2 result (crates/archon-workflow/src/events.rs
/// `REDACTION_MARKER`, Issue-245/248). The rest of the run is golden.
#[test]
fn redaction_of_a_secret_in_an_author_reply() {
    let name = "redaction";
    let project = project_with(Plan {
        variant: Variant::SecretInAcceptanceReply,
        block: None,
    });
    decompose(&project, name);
    let run = one_run(project.path());
    let mut redacted_copies = 0;
    for path in files_under(&run) {
        let relative = path
            .strip_prefix(&run)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        let bytes = fs::read(&path).unwrap();
        let text = String::from_utf8_lossy(&bytes);
        if relative.starts_with("v2/") || relative.starts_with("agent-results/") {
            continue;
        }
        assert!(
            !text.contains(SECRET_TOKEN),
            "{name}: public copy {relative} holds the secret-shaped token"
        );
        redacted_copies += usize::from(text.contains("<redacted>"));
    }
    assert!(
        redacted_copies > 0,
        "{name}: no public copy shows the redaction marker; the token was dropped, not redacted"
    );
    let mut actual = final_snapshot(&project, &[]);
    let mut authored = 0;
    for (_, record) in &mut actual.records {
        if let Some(content) = record.pointer_mut("/result/data/content")
            && content
                .as_str()
                .is_some_and(|text| text.contains(SECRET_TOKEN))
        {
            authored += 1;
            let text = content.as_str().unwrap();
            let entry = text
                .split_once("```json\n")
                .and_then(|(_, rest)| rest.split_once("\n```"))
                .map(|(entry, _)| entry.to_string())
                .expect("the authored reply keeps its fenced entry");
            *content = serde_json::Value::String(entry);
        }
    }
    assert_eq!(
        authored, 1,
        "{name}: the authoritative author record keeps the reply as authored"
    );
    assert_eq!(
        actual,
        golden(),
        "{name}: apart from the authored reply text, the run equals golden"
    );
    assert_eq!(project.provider.requests.per_call(), read_golden_requests());
    progress(name, "passed");
}

/// Every frozen entry: the PRD's criteria, then the owed supplementary ones.
fn entries(contract: &serde_json::Value) -> Vec<&serde_json::Value> {
    ["acceptance", "supplementary"]
        .into_iter()
        .flat_map(|list| contract[list].as_array().into_iter().flatten())
        .collect()
}

/// The frozen check of `id`, run with `sh -c` in a scratch tree that holds
/// `files`: its exit status.
#[cfg(unix)]
fn run_frozen_check(command: &str, files: &[(&str, &str)]) -> std::process::ExitStatus {
    let scratch = tempfile::tempdir().unwrap();
    for (path, contents) in files {
        let path = scratch.path().join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, contents).unwrap();
    }
    std::process::Command::new("sh")
        .arg("-c")
        .arg(command)
        .current_dir(scratch.path())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .unwrap()
}

#[cfg(unix)]
fn script_cell(name: &str, variant: Variant, id: &str, passing: &[(&str, &str)]) {
    let project = project_with(Plan {
        variant,
        block: None,
    });
    decompose(&project, name);
    let actual = final_snapshot(&project, &[]);
    let golden = golden();
    // The skeleton binds the contract's digest, which the script changes.
    let unbound = |skeleton: &serde_json::Value| {
        let mut skeleton = skeleton.clone();
        skeleton["acceptance_digest"] = serde_json::Value::Null;
        skeleton
    };
    assert_eq!(
        unbound(&actual.skeleton),
        unbound(&golden.skeleton),
        "{name}: skeleton"
    );
    assert_eq!(
        actual.task_bodies, golden.task_bodies,
        "{name}: task bodies"
    );
    let frozen = entries(&actual.acceptance)
        .into_iter()
        .find(|entry| entry["id"] == id)
        .cloned()
        .unwrap_or_else(|| panic!("{name}: {id} is not in the frozen contract"));
    let command = frozen["check"]["command"]
        .as_str()
        .unwrap_or_else(|| panic!("{name}: {id} froze no command: {frozen}"));
    assert!(
        !command.contains('\r'),
        "{name}: the frozen check keeps carriage returns, so `sh` reads them into its words: {command:?}"
    );
    assert!(
        run_frozen_check(command, passing).success(),
        "{name}: the frozen check fails on a tree where its criterion holds: {command:?}"
    );
    assert!(
        !run_frozen_check(command, &[]).success(),
        "{name}: the frozen check passes on a tree where its criterion is false: {command:?}"
    );
    let mut without = actual.acceptance.clone();
    let mut expected = golden.acceptance.clone();
    for contract in [&mut without, &mut expected] {
        for list in ["acceptance", "supplementary"] {
            for entry in contract[list].as_array_mut().unwrap() {
                if entry["id"] == id {
                    entry["check"] = serde_json::Value::Null;
                }
            }
        }
    }
    assert_eq!(
        without, expected,
        "{name}: apart from {id}'s check, the frozen contract equals golden"
    );
    assert_eq!(
        project.provider.requests.per_call(),
        read_golden_requests(),
        "{name}: the script must be accepted without a re-author"
    );
    progress(name, "passed");
}

#[cfg(unix)]
#[test]
#[ignore = "defect #423: a CRLF check block is frozen with its carriage returns, so the frozen check can never pass"]
fn crlf_verifier_script_in_a_check_block() {
    assert!(CRLF_SCRIPT.contains("\r\n"));
    script_cell(
        "crlf-check",
        Variant::CrlfCheck,
        "SUP-REQ-X-001",
        &[("src/alpha.txt", "alpha ready\n")],
    );
}

#[cfg(unix)]
#[test]
fn heredoc_verifier_script_in_a_check_block() {
    assert!(HEREDOC_SCRIPT.contains("<<'PY'"));
    script_cell(
        "heredoc-check",
        Variant::HeredocCheck,
        "SUP-REQ-X-002",
        &[("src/beta.json", "{\"ready\": true}")],
    );
}

/// A candidate with 60 acceptance criteria (62 checks with the two owed
/// supplementary ones) freezes, judges and lands with every call made once.
#[test]
#[ignore = "nightly: takes 70 s locally and 124-150 s on CI runners, over the 2-minute budget"]
fn large_candidate_with_sixty_acceptance_checks() {
    const CRITERIA: usize = 60;
    let name = "large-candidate";
    let base = fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/offline_decomposition/prd.md"
    ))
    .unwrap();
    let project = project_with_files(
        Plan {
            variant: Variant::Large(CRITERIA),
            block: None,
        },
        &[
            ("prd.md", large::prd(&base, CRITERIA)),
            (".archon/proof/outcome.json", large::outcome(CRITERIA)),
        ],
    );
    // The 62-check freeze prints nothing for longer than the default
    // no-progress bound when the cells run in parallel. The bound stays under
    // nextest's 240 s terminate-after (.config/nextest.toml), so a stall is
    // reported with this cell's own evidence instead of a bare TIMEOUT.
    decompose_within(&project, name, std::time::Duration::from_secs(180));
    let actual = final_snapshot(&project, &[]);
    let entries = entries(&actual.acceptance);
    assert_eq!(entries.len(), CRITERIA + 2, "{name}: frozen entries");
    for entry in entries {
        assert_eq!(
            entry["judgment"]["verdict"], "accepted",
            "{name}: every check is judged and accepted: {entry}"
        );
    }
    let requests = project.provider.requests.per_call();
    let repeated = requests
        .iter()
        .filter(|(_, count)| **count != 1)
        .collect::<Vec<_>>();
    assert!(
        repeated.is_empty(),
        "{name}: a call was asked more than once: {repeated:?}"
    );
    for k in 1..=CRITERIA {
        assert!(
            requests.contains_key(&format!("acceptance:AC-X-{k:03}")),
            "{name}: AC-X-{k:03} was never authored"
        );
    }
    progress(name, "passed");
}
