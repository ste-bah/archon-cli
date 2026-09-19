//! Obs-8: the next session is told what the previous one was refused and
//! last ran, bounded, and never file contents.
use super::*;
use crate::WorkflowV2BranchOutcome;
use crate::v2::{WorkflowV2Result, WorkflowV2Status};
use serde_json::json;

/// The guard's refusals as the guard records them: the first line, clipped
/// to its 120-character record head (the live texts run longer).
const RELEASE_REFUSAL: &str = "Release builds are disabled for this write-capable workflow call. Use cargo check -p <crate> and focused tests; the ope…";
const WORKTREE_REFUSAL: &str = "git worktree is refused: git history/worktree mutation is host-owned in workflow runs — the write coordinator commits y…";

fn sidecar(store: &WorkflowV2ResultStore, call_id: &str, records: &[Value]) {
    let path = crate::v2::write_read_set::path(store, call_id);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let text: String = records.iter().map(|r| format!("{r}\n")).collect();
    std::fs::write(path, text).unwrap();
}

fn refusal(call: u64, head: &str, reason: &str) -> Value {
    assert!(
        reason.chars().count() <= 120,
        "fixture reasons are guard-clipped"
    );
    json!({"kind":"refusal","call":call,"tool":"Bash","head":head,"reason":reason})
}

fn call(call: u64, tool: &str, head: &str, status: &str) -> Value {
    json!({"kind":"tool_call","call":call,"tool":tool,"head":head,"status":status})
}

/// The live shape: the first session's refusals and its trail, as the guard
/// records them, mixed with the read-range records that share the file.
fn first_session() -> Vec<Value> {
    vec![
        json!({"path":"src/lib.rs","offset":0,"limit":40,"call":1,"hash":"h"}),
        call(1, "Read", "src/lib.rs", "ok"),
        refusal(2, "cargo build --release", RELEASE_REFUSAL),
        call(
            2,
            "Bash",
            "cargo build --release",
            &format!("refused: {RELEASE_REFUSAL}"),
        ),
        refusal(3, "git worktree add /tmp/dl002-base HEAD", WORKTREE_REFUSAL),
        call(
            3,
            "Bash",
            "git worktree add /tmp/dl002-base HEAD",
            &format!("refused: {WORKTREE_REFUSAL}"),
        ),
        call(
            4,
            "Bash",
            "git archive HEAD | tar -x -C /tmp/base",
            "exit 0",
        ),
        call(5, "Bash", "cargo check -p archon-workflow", "exit 101"),
    ]
}

#[test]
fn a_refusal_recorded_in_attempt_one_reaches_the_retry_and_restart_preambles() {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowV2ResultStore::new(temp.path().join("v2"));
    sidecar(&store, "agents-4-0", &first_session());
    let memory = SessionMemory::for_branch(&store, "agents-4-0", DEFAULT_LAST_CALLS);
    assert_eq!(memory.refusals.len(), 2, "{memory:#?}");
    assert_eq!(
        memory.refusals[0],
        format!("Bash `cargo build --release` → {RELEASE_REFUSAL}")
    );
    assert_eq!(
        memory.refusals[1],
        format!("Bash `git worktree add /tmp/dl002-base HEAD` → {WORKTREE_REFUSAL}")
    );
    assert_eq!(memory.last_calls.len(), 5, "{memory:#?}");
    assert_eq!(memory.last_calls[0], "Read `src/lib.rs` → ok");
    assert_eq!(
        memory.last_calls[3],
        "Bash `git archive HEAD | tar -x -C /tmp/base` → exit 0"
    );
    assert_eq!(
        memory.last_calls[4],
        "Bash `cargo check -p archon-workflow` → exit 101"
    );

    let partial = super::super::partial_work::PartialWork {
        patch_path: "p".into(),
        files: vec!["src/lib.rs".into()],
        bytes: 1,
        baseline_commit: "c".into(),
        origin: None,
    };
    // The in-run retry / cross-wave resume preamble.
    let retry = super::super::partial_work::with_host_preamble(
        "do the task",
        Some(std::time::Duration::from_secs(1800)),
        Some(&partial),
        &memory,
    );
    let section = memory.render().unwrap();
    assert!(section.starts_with("The previous session had these tool calls refused by the host — do not retry them:\n  - Bash `cargo build --release` → Release builds are disabled"), "{section}");
    assert!(
        section.contains(
            "\nIts last 5 tool calls (most recent last) were:\n  - Read `src/lib.rs` → ok\n"
        ),
        "{section}"
    );
    assert!(
        retry.contains("has been applied to this workspace: src/lib.rs"),
        "{retry}"
    );
    let partial_at = retry.find("A previous attempt at this task").unwrap();
    let memory_at = retry.find(&section).expect("section rendered verbatim");
    assert!(
        partial_at < memory_at,
        "memory follows the partial sentence:\n{retry}"
    );
    assert!(retry.ends_with("\n\ndo the task"), "{retry}");
    // The mid-attempt restart preamble.
    let restart = super::super::partial_work::with_restart_preamble(
        "do the task",
        None,
        Some(&partial),
        &memory,
    );
    assert!(restart.contains("same attempt, restarted"), "{restart}");
    assert!(restart.contains(&section), "{restart}");
    // Nothing to say renders nothing.
    assert_eq!(
        super::super::partial_work::with_host_preamble(
            "do the task",
            None,
            None,
            &SessionMemory::default()
        ),
        "do the task"
    );
    assert_eq!(
        SessionMemory::for_branch(&store, "never-ran", 12),
        SessionMemory::default()
    );
}

#[test]
fn identical_refusals_are_shown_once_and_the_lists_are_bounded() {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowV2ResultStore::new(temp.path().join("v2"));
    let mut records = Vec::new();
    // The same refusal 30 times, then 30 distinct ones, then 100 calls.
    for n in 0..30 {
        records.push(refusal(n, "cargo build --release", RELEASE_REFUSAL));
    }
    for n in 0..30 {
        records.push(refusal(
            100 + n,
            &format!("git stash {n}"),
            "git stash is refused",
        ));
    }
    let long_head = "x".repeat(400);
    for n in 0..100u64 {
        records.push(call(200 + n, "Bash", &format!("{long_head} {n}"), "exit 0"));
    }
    sidecar(&store, "b", &records);
    let memory = SessionMemory::for_branch(&store, "b", 12);
    assert_eq!(memory.refusals.len(), MAX_REFUSALS);
    assert_eq!(
        memory.refusals[0],
        format!("Bash `cargo build --release` → {RELEASE_REFUSAL}")
    );
    assert_eq!(
        memory.refusals[1],
        "Bash `git stash 0` → git stash is refused"
    );
    assert_eq!(
        memory
            .refusals
            .iter()
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
        MAX_REFUSALS,
        "deduplicated"
    );
    assert_eq!(memory.last_calls.len(), 12);
    assert!(
        memory
            .last_calls
            .iter()
            .all(|line| line.chars().count() <= MAX_LINE_CHARS),
        "{memory:#?}"
    );
    assert!(
        memory.last_calls[11].ends_with('\u{2026}'),
        "clipped: {}",
        memory.last_calls[11]
    );
    // Most recent last: the twelve kept are calls 88..=99.
    let kept_first = &memory.last_calls[0];
    assert!(kept_first.starts_with("Bash `xxx"), "{kept_first}");
    // The operator's number is honoured up to the ceiling, and zero shows no trail.
    assert_eq!(
        SessionMemory::for_branch(&store, "b", 3).last_calls.len(),
        3
    );
    assert_eq!(
        SessionMemory::for_branch(&store, "b", 500).last_calls.len(),
        MAX_LAST_CALLS
    );
    let none = SessionMemory::for_branch(&store, "b", 0);
    assert!(none.last_calls.is_empty());
    assert!(none.render().unwrap().contains("do not retry them"));
}

/// A fresh attempt in a later wave finds the earlier branch through the
/// saved outcome's task ids, the way the read set and the partial do.
#[test]
fn a_later_wave_resuming_the_task_sees_the_earlier_branch_memory() {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowV2ResultStore::new(temp.path().join("v2"));
    sidecar(&store, "agents-4-0", &first_session());
    sidecar(&store, "agents-9-0", &[call(1, "Read", "other.rs", "ok")]);
    for (stage, item, task) in [
        ("agents-4", "agents-4-0", "TASK-001"),
        ("agents-9", "agents-9-0", "TASK-002"),
    ] {
        let result = WorkflowV2Result {
            status: WorkflowV2Status::NeedsReview,
            data: json!({"canonical_task_ids":[task]}),
            ..Default::default()
        };
        store
            .save_branch_outcome(
                stage,
                &WorkflowV2BranchOutcome {
                    item_id: item.into(),
                    role: "coder".into(),
                    status: result.status,
                    result: Some(result),
                    error: None,
                    failure_kind: None,
                    item_input_hash: None,
                    completion_evidence: Vec::new(),
                },
            )
            .unwrap();
    }
    let memory = SessionMemory::for_tasks(&store, &["TASK-001".into()], 2);
    assert_eq!(memory.refusals.len(), 2, "{memory:#?}");
    assert_eq!(
        memory.last_calls,
        vec![
            "Bash `git archive HEAD | tar -x -C /tmp/base` → exit 0".to_string(),
            "Bash `cargo check -p archon-workflow` → exit 101".to_string(),
        ]
    );
    assert!(SessionMemory::for_tasks(&store, &["TASK-003".into()], 12).is_empty());
    assert!(SessionMemory::for_tasks(&store, &[], 12).is_empty());
}

/// Issue-54: the guard's terminal refusal reaches the next attempt with its
/// count, whatever else the session was refused — even past the cap on
/// distinct refusals, which a thrashing session's refused reads fill.
#[test]
fn a_session_the_host_ended_for_read_wall_thrash_says_so_first() {
    const TERMINAL: &str = "read-wall thrash: 16 non-writing calls after the read budget was exhausted; 0 substantive writes";
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowV2ResultStore::new(temp.path().join("v2"));
    let mut records = first_session();
    // The live shape: more distinct refused reads than the cap keeps, then
    // the terminal refusal repeated under different heads.
    for n in 0..(MAX_REFUSALS as u64 + 5) {
        let head = format!("sed -n '{},{}p' src/lib.rs", n * 40 + 1, n * 40 + 40);
        records.push(refusal(10 + n, &head, "read budget exhausted (55 reads, 0 substantive writes). Write a deliverable file now; each successful substantive…"));
    }
    records.push(refusal(90, "grep -c \"\" src/lib.rs", TERMINAL));
    records.push(call(
        90,
        "Bash",
        "grep -c \"\" src/lib.rs",
        &format!("refused: {TERMINAL}"),
    ));
    records.push(refusal(91, "echo uv", TERMINAL));
    records.push(call(91, "Bash", "echo uv", &format!("refused: {TERMINAL}")));
    sidecar(&store, "agents-5-0", &records);

    let memory = SessionMemory::for_branch(&store, "agents-5-0", 2);
    assert_eq!(memory.ended_by_host.as_deref(), Some(TERMINAL));
    assert_eq!(memory.refusals.len(), MAX_REFUSALS, "{memory:#?}");
    assert!(
        memory
            .refusals
            .iter()
            .all(|line| !line.contains("read-wall thrash")),
        "the terminal refusal is carried once, in its own field: {memory:#?}"
    );
    let text = memory.render().unwrap();
    assert!(
        text.starts_with(&format!(
            "The host ended the previous session ({TERMINAL})."
        )),
        "{text}"
    );
    assert!(
        text.contains("write or edit a deliverable file first"),
        "{text}"
    );
    assert!(text.contains("do not retry them"), "{text}");
    assert!(text.ends_with("Bash `echo uv` → refused: read-wall thrash: 16 non-writing calls after the read budget was exhausted; 0 substantive writes"), "{text}");

    // The note alone is a memory worth rendering.
    let only = SessionMemory {
        ended_by_host: Some(TERMINAL.into()),
        ..Default::default()
    };
    assert!(!only.is_empty());
    assert_eq!(
        only.render().unwrap(),
        format!(
            "The host ended the previous session ({TERMINAL}). Reading past the budget does not help: write or edit a deliverable file first, then run the declared tests."
        )
    );
}
