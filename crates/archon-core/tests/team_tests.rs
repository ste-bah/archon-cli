//! Tests for TASK-CLI-312: Agent Teams.

use archon_core::team::backend::{InMemoryBackend, InboxBackend};
use archon_core::team::message::{MessageType, TeamMessage};
use archon_core::team::team_config::{MemberConfig, TeamConfig};

// ---------------------------------------------------------------------------
// TeamConfig serialization
// ---------------------------------------------------------------------------

#[test]
fn team_config_serializes_to_json() {
    let config = TeamConfig {
        id: "team-001".to_string(),
        name: "test team".to_string(),
        members: vec![
            MemberConfig {
                role: "coder".to_string(),
                system_prompt: "You write code.".to_string(),
                model: None,
                tools: vec![],
            },
            MemberConfig {
                role: "reviewer".to_string(),
                system_prompt: "You review code.".to_string(),
                model: Some("claude-sonnet-4-6".to_string()),
                tools: vec!["Read".to_string()],
            },
        ],
    };
    let json = serde_json::to_string(&config).unwrap();
    assert!(json.contains("team-001"));
    assert!(json.contains("coder"));
    assert!(json.contains("reviewer"));
}

#[test]
fn team_config_round_trips_json() {
    let config = TeamConfig {
        id: "abc".to_string(),
        name: "my team".to_string(),
        members: vec![MemberConfig {
            role: "agent1".to_string(),
            system_prompt: "Do stuff.".to_string(),
            model: None,
            tools: vec![],
        }],
    };
    let json = serde_json::to_string(&config).unwrap();
    let restored: TeamConfig = serde_json::from_str(&json).unwrap();
    assert_eq!(restored.id, "abc");
    assert_eq!(restored.members.len(), 1);
    assert_eq!(restored.members[0].role, "agent1");
}

// ---------------------------------------------------------------------------
// MessageType enum
// ---------------------------------------------------------------------------

#[test]
fn message_type_all_variants_exist() {
    fn _check(mt: MessageType) -> u8 {
        match mt {
            MessageType::Chat => 1,
            MessageType::TaskAssignment => 2,
            MessageType::StatusUpdate => 3,
            MessageType::Completion => 4,
            MessageType::Error => 5,
        }
    }
    // All 5 variants must exist — compile-time proof
    assert_eq!(_check(MessageType::Chat), 1);
}

#[test]
fn team_message_serializes() {
    let msg = TeamMessage {
        from: "coder".to_string(),
        to: "reviewer".to_string(),
        content: "PR is ready".to_string(),
        timestamp: 1000,
        message_type: MessageType::StatusUpdate,
    };
    let json = serde_json::to_string(&msg).unwrap();
    assert!(json.contains("coder"));
    assert!(json.contains("reviewer"));
    assert!(json.contains("StatusUpdate"));
}

// ---------------------------------------------------------------------------
// InMemoryBackend
// ---------------------------------------------------------------------------

#[test]
fn in_memory_backend_send_and_receive() {
    let mut backend = InMemoryBackend::new();
    let msg = TeamMessage {
        from: "agent1".to_string(),
        to: "agent2".to_string(),
        content: "hello".to_string(),
        timestamp: 0,
        message_type: MessageType::Chat,
    };
    backend.send("agent2", msg.clone()).unwrap();
    let received = backend.read_and_clear("agent2").unwrap();
    assert_eq!(received.len(), 1);
    assert_eq!(received[0].content, "hello");
}

#[test]
fn in_memory_backend_read_clears_inbox() {
    let mut backend = InMemoryBackend::new();
    backend
        .send(
            "r1",
            TeamMessage {
                from: "s".to_string(),
                to: "r1".to_string(),
                content: "msg".to_string(),
                timestamp: 0,
                message_type: MessageType::Chat,
            },
        )
        .unwrap();
    let first = backend.read_and_clear("r1").unwrap();
    assert_eq!(first.len(), 1);
    let second = backend.read_and_clear("r1").unwrap();
    assert!(second.is_empty(), "after reading, inbox must be empty");
}

#[test]
fn in_memory_backend_send_to_all() {
    let backend = InMemoryBackend::new();
    let roles = ["coder", "tester", "reviewer"];
    for role in roles {
        backend.register(role);
    }
    backend
        .send_to_all("coordinator", "kickoff", MessageType::TaskAssignment)
        .unwrap();
    for role in roles {
        let msgs = backend.read_and_clear(role).unwrap();
        assert_eq!(msgs.len(), 1, "each member should have received 1 message");
        assert_eq!(msgs[0].message_type, MessageType::TaskAssignment);
    }
}

#[test]
fn in_memory_backend_empty_inbox_returns_empty_vec() {
    let backend = InMemoryBackend::new();
    let msgs = backend.read_and_clear("nonexistent").unwrap();
    assert!(msgs.is_empty());
}

// ---------------------------------------------------------------------------
// FileBasedBackend
// ---------------------------------------------------------------------------

#[test]
fn file_based_backend_send_and_receive() {
    let tmp = tempfile::tempdir().unwrap();
    let mut backend = archon_core::team::backend::FileBasedBackend::new(tmp.path().to_path_buf());

    let msg = TeamMessage {
        from: "writer".to_string(),
        to: "reader".to_string(),
        content: "test message".to_string(),
        timestamp: 42,
        message_type: MessageType::Chat,
    };
    backend.send("reader", msg).unwrap();

    let inbox_path = tmp.path().join("inbox-reader.jsonl");
    assert!(inbox_path.exists(), "inbox file must be created");

    let received = backend.read_and_clear("reader").unwrap();
    assert_eq!(received.len(), 1);
    assert_eq!(received[0].content, "test message");
}

#[test]
fn file_based_backend_read_clears_file() {
    let tmp = tempfile::tempdir().unwrap();
    let mut backend = archon_core::team::backend::FileBasedBackend::new(tmp.path().to_path_buf());
    backend
        .send(
            "target",
            TeamMessage {
                from: "src".to_string(),
                to: "target".to_string(),
                content: "x".to_string(),
                timestamp: 0,
                message_type: MessageType::Chat,
            },
        )
        .unwrap();
    backend.read_and_clear("target").unwrap();
    let second = backend.read_and_clear("target").unwrap();
    assert!(second.is_empty(), "file must be cleared after read");
}

#[test]
fn file_based_backend_persists_across_instances() {
    let tmp = tempfile::tempdir().unwrap();
    {
        let mut backend =
            archon_core::team::backend::FileBasedBackend::new(tmp.path().to_path_buf());
        backend
            .send(
                "member",
                TeamMessage {
                    from: "other".to_string(),
                    to: "member".to_string(),
                    content: "persisted".to_string(),
                    timestamp: 99,
                    message_type: MessageType::Completion,
                },
            )
            .unwrap();
    }
    // New instance reads the file
    let backend2 = archon_core::team::backend::FileBasedBackend::new(tmp.path().to_path_buf());
    let msgs = backend2.read_and_clear("member").unwrap();
    assert_eq!(msgs.len(), 1);
    assert_eq!(msgs[0].content, "persisted");
}

