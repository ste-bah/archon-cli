//! Issue #171 Part 4 — the transcript store holds one append writer per agent
//! transcript instead of reopening the file for every message.
//!
//! The syscall-level claim (one `open` per agent transcript) is evidenced by an
//! strace run over this same code; these assertions cover the observable
//! contract: open count, per-message durability, and byte-for-byte identical
//! transcript content.

use super::*;
use tempfile::TempDir;

fn store() -> (AgentTranscriptStore, TempDir) {
    let tmp = TempDir::new().unwrap();
    (
        AgentTranscriptStore::with_base_dir(tmp.path().to_path_buf()),
        tmp,
    )
}

#[test]
fn opens_transcript_file_once_across_many_messages() {
    let (store, _tmp) = store();
    for i in 0..25 {
        store.record_message("agent-1", &serde_json::json!({"seq": i}));
    }
    assert_eq!(
        store.open_count(),
        1,
        "expected one open for the whole agent transcript"
    );
    assert_eq!(store.get_transcript("agent-1").unwrap().len(), 25);
}

#[test]
fn opens_once_per_agent_not_once_per_store() {
    let (store, _tmp) = store();
    for i in 0..5 {
        store.record_message("agent-a", &serde_json::json!({"seq": i}));
        store.record_message("agent-b", &serde_json::json!({"seq": i}));
    }
    assert_eq!(store.open_count(), 2, "one open per distinct agent id");
    assert_eq!(store.get_transcript("agent-a").unwrap().len(), 5);
    assert_eq!(store.get_transcript("agent-b").unwrap().len(), 5);
}

#[test]
fn clones_share_the_held_writer() {
    let (store, _tmp) = store();
    let clone = store.clone();
    store.record_message("agent-1", &serde_json::json!({"n": 1}));
    clone.record_message("agent-1", &serde_json::json!({"n": 2}));
    assert_eq!(store.open_count(), 1);
    assert_eq!(clone.open_count(), 1);
    assert_eq!(store.get_transcript("agent-1").unwrap().len(), 2);
}

#[test]
fn each_message_is_flushed_before_record_returns() {
    let (store, _tmp) = store();
    // Reading through a separate handle only sees flushed bytes. If the writer
    // batched across messages this would observe fewer lines than were written.
    for i in 1..=10 {
        store.record_message("agent-1", &serde_json::json!({"seq": i}));
        let seen = std::fs::read_to_string(store.transcript_path("agent-1")).unwrap();
        assert_eq!(
            seen.lines().filter(|l| !l.trim().is_empty()).count(),
            i,
            "message {i} was not durable when record_message returned"
        );
    }
}

/// Byte-identical to the pre-change (`OpenOptions` + `writeln!` per message)
/// encoding: compact JSON, one message per line, `\n` terminated, no trailing
/// blank line.
#[test]
fn bytes_match_the_reopen_per_message_fixture() {
    let (store, _tmp) = store();
    let messages = vec![
        serde_json::json!({"role": "user", "content": "hello"}),
        serde_json::json!({"role": "assistant", "content": [{"type": "text", "text": "hi"}]}),
        serde_json::json!({"role": "user", "content": "unicode: \u{e9}\u{4e2d}"}),
    ];

    let expected: String = messages
        .iter()
        .map(|m| format!("{}\n", serde_json::to_string(m).unwrap()))
        .collect();

    for m in &messages {
        store.record_message("fixture", m);
    }

    let actual = std::fs::read(store.transcript_path("fixture")).unwrap();
    assert_eq!(actual, expected.as_bytes());
}

#[test]
fn append_continues_an_existing_transcript_file() {
    let tmp = TempDir::new().unwrap();
    let first = AgentTranscriptStore::with_base_dir(tmp.path().to_path_buf());
    first.record_message("agent-1", &serde_json::json!({"n": 1}));
    drop(first);

    let second = AgentTranscriptStore::with_base_dir(tmp.path().to_path_buf());
    second.record_message("agent-1", &serde_json::json!({"n": 2}));

    let transcript = second.get_transcript("agent-1").unwrap();
    assert_eq!(transcript.len(), 2);
    assert_eq!(transcript[0]["n"], 1);
    assert_eq!(transcript[1]["n"], 2);
}

#[test]
fn open_failure_is_not_cached_and_does_not_panic() {
    // A file where the base directory should be: `create_dir_all` fails, so the
    // open never happens and nothing is memoized.
    let tmp = TempDir::new().unwrap();
    let blocker = tmp.path().join("blocked");
    std::fs::write(&blocker, b"not a directory").unwrap();

    let store = AgentTranscriptStore::with_base_dir(blocker);
    store.record_message("agent-1", &serde_json::json!({"n": 1}));
    store.record_message("agent-1", &serde_json::json!({"n": 2}));

    assert_eq!(store.open_count(), 0);
    assert!(store.get_transcript("agent-1").is_none());
}
