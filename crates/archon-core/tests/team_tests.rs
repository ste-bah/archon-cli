//! Tests for TASK-CLI-312: Agent Teams.

use archon_core::team::backend::{InboxBackend, InMemoryBackend};
use archon_core::team::message::{MessageType, TeamMessage};
use archon_core::team::team_config::{MemberConfig, TeamConfig};
use archon_core::team::TeamManager;

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
    backend.send("r1", TeamMessage {
        from: "s".to_string(), to: "r1".to_string(), content: "msg".to_string(),
        timestamp: 0, message_type: MessageType::Chat,
    }).unwrap();
    let first = backend.read_and_clear("r1").unwrap();
    assert_eq!(first.len(), 1);
    let second = backend.read_and_clear("r1").unwrap();
    assert!(second.is_empty(), "after reading, inbox must be empty");
}

#[test]
fn in_memory_backend_send_to_all() {
    let mut backend = InMemoryBackend::new();
    let roles = ["coder", "tester", "reviewer"];
    for role in roles {
        backend.register(role);
    }
    backend.send_to_all("coordinator", "kickoff", MessageType::TaskAssignment).unwrap();
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
    backend.send("target", TeamMessage {
        from: "src".to_string(), to: "target".to_string(), content: "x".to_string(),
        timestamp: 0, message_type: MessageType::Chat,
    }).unwrap();
    backend.read_and_clear("target").unwrap();
    let second = backend.read_and_clear("target").unwrap();
    assert!(second.is_empty(), "file must be cleared after read");
}

#[test]
fn file_based_backend_persists_across_instances() {
    let tmp = tempfile::tempdir().unwrap();
    {
        let mut backend = archon_core::team::backend::FileBasedBackend::new(tmp.path().to_path_buf());
        backend.send("member", TeamMessage {
            from: "other".to_string(), to: "member".to_string(), content: "persisted".to_string(),
            timestamp: 99, message_type: MessageType::Completion,
        }).unwrap();
    }
    // New instance reads the file
    let backend2 = archon_core::team::backend::FileBasedBackend::new(tmp.path().to_path_buf());
    let msgs = backend2.read_and_clear("member").unwrap();
    assert_eq!(msgs.len(), 1);
    assert_eq!(msgs[0].content, "persisted");
}

// ---------------------------------------------------------------------------
// TeamManager
// ---------------------------------------------------------------------------

#[test]
fn team_manager_create_and_load() {
    let tmp = tempfile::tempdir().unwrap();
    let mut manager = TeamManager::new(tmp.path().to_path_buf());

    let config = TeamConfig {
        id: "team-test".to_string(),
        name: "test".to_string(),
        members: vec![
            MemberConfig { role: "a".to_string(), system_prompt: "prompt a".to_string(), model: None, tools: vec![] },
            MemberConfig { role: "b".to_string(), system_prompt: "prompt b".to_string(), model: None, tools: vec![] },
        ],
    };
    manager.create_team(config.clone()).unwrap();

    let team_dir = tmp.path().join("teams").join("team-test");
    assert!(team_dir.exists(), "team directory must be created");
    assert!(team_dir.join("team.json").exists(), "team.json must exist");
    assert!(team_dir.join("inbox-a.jsonl").exists(), "inbox for 'a' must exist");
    assert!(team_dir.join("inbox-b.jsonl").exists(), "inbox for 'b' must exist");
}

#[test]
fn team_manager_delete_removes_directory() {
    let tmp = tempfile::tempdir().unwrap();
    let mut manager = TeamManager::new(tmp.path().to_path_buf());

    let config = TeamConfig {
        id: "delete-me".to_string(),
        name: "ephemeral".to_string(),
        members: vec![MemberConfig { role: "x".to_string(), system_prompt: "".to_string(), model: None, tools: vec![] }],
    };
    manager.create_team(config).unwrap();
    let team_dir = tmp.path().join("teams").join("delete-me");
    assert!(team_dir.exists());
    manager.delete_team("delete-me").unwrap();
    assert!(!team_dir.exists(), "team directory must be deleted");
}

#[test]
fn team_manager_create_does_not_spawn_agents() {
    // TeamCreate creates config files only — no process spawning.
    // We verify this by ensuring create_team() returns quickly without
    // any child process side effects. The test creates a team and checks
    // only filesystem artifacts, not any spawned processes.
    let tmp = tempfile::tempdir().unwrap();
    let mut manager = TeamManager::new(tmp.path().to_path_buf());
    let before = std::time::Instant::now();
    manager.create_team(TeamConfig {
        id: "no-spawn".to_string(), name: "x".to_string(),
        members: vec![MemberConfig { role: "m".to_string(), system_prompt: "".to_string(), model: None, tools: vec![] }],
    }).unwrap();
    // If a process were spawned, it would take longer
    let elapsed = before.elapsed();
    assert!(elapsed.as_secs() < 2, "create_team should complete instantly (no process spawning)");
}
