//! Tests for TASK-CLI-312 team tools: TeamCreate, TeamDelete, SendMessage, ReadTeamMessages.

use archon_tools::read_team_messages::ReadTeamMessagesTool;
use archon_tools::send_message_team::SendMessageTeamTool;
use archon_tools::team_create::TeamCreateTool;
use archon_tools::team_delete::TeamDeleteTool;
use archon_tools::tool::{AgentMode, Tool, ToolContext};

fn ctx(project_dir: &std::path::Path) -> ToolContext {
    ToolContext {
        working_dir: project_dir.to_path_buf(),
        session_id: "test-session".to_string(),
        mode: AgentMode::Normal,
    }
}

// ---------------------------------------------------------------------------
// TeamCreate
// ---------------------------------------------------------------------------

#[tokio::test]
async fn team_create_tool_creates_files() {
    let tmp = tempfile::tempdir().unwrap();
    let tool = TeamCreateTool::new(tmp.path().to_path_buf());
    let result = tool
        .execute(
            serde_json::json!({
                "name": "test-team",
                "members": [
                    { "role": "coder", "system_prompt": "Write code." },
                    { "role": "tester", "system_prompt": "Write tests." }
                ]
            }),
            &ctx(tmp.path()),
        )
        .await;

    assert!(
        !result.is_error,
        "TeamCreate must succeed: {}",
        result.content
    );
    let output: serde_json::Value = serde_json::from_str(&result.content).unwrap();
    let team_id = output["team_id"].as_str().unwrap();
    assert!(!team_id.is_empty());

    let team_dir = tmp.path().join(".claude").join("teams").join(team_id);
    assert!(
        team_dir.join("team.json").exists(),
        "team.json must be created"
    );
    assert!(
        team_dir.join("inbox-coder.jsonl").exists(),
        "coder inbox must exist"
    );
    assert!(
        team_dir.join("inbox-tester.jsonl").exists(),
        "tester inbox must exist"
    );
}

#[tokio::test]
async fn team_create_returns_team_id_and_roles() {
    let tmp = tempfile::tempdir().unwrap();
    let tool = TeamCreateTool::new(tmp.path().to_path_buf());
    let result = tool
        .execute(
            serde_json::json!({
                "name": "my-team",
                "members": [
                    { "role": "a", "system_prompt": "." },
                    { "role": "b", "system_prompt": "." }
                ]
            }),
            &ctx(tmp.path()),
        )
        .await;
    assert!(!result.is_error);
    let output: serde_json::Value = serde_json::from_str(&result.content).unwrap();
    let roles = output["roles"].as_array().unwrap();
    assert_eq!(roles.len(), 2);
    assert!(roles.iter().any(|r| r.as_str() == Some("a")));
    assert!(roles.iter().any(|r| r.as_str() == Some("b")));
}

#[tokio::test]
async fn team_create_does_not_spawn_processes() {
    // TeamCreate is config-only — it must not spawn agent processes.
    // We verify by timing: file creation should be near-instant.
    let tmp = tempfile::tempdir().unwrap();
    let tool = TeamCreateTool::new(tmp.path().to_path_buf());
    let before = std::time::Instant::now();
    tool.execute(
        serde_json::json!({
            "name": "fast-team",
            "members": [{ "role": "m", "system_prompt": "." }]
        }),
        &ctx(tmp.path()),
    )
    .await;
    assert!(before.elapsed().as_secs() < 2);
}

// ---------------------------------------------------------------------------
// SendMessage + ReadTeamMessages
// ---------------------------------------------------------------------------

#[tokio::test]
async fn send_and_read_messages() {
    let tmp = tempfile::tempdir().unwrap();

    // Create a team first
    let create_tool = TeamCreateTool::new(tmp.path().to_path_buf());
    let create_result = create_tool
        .execute(
            serde_json::json!({
                "name": "msg-team",
                "members": [
                    { "role": "sender", "system_prompt": "." },
                    { "role": "receiver", "system_prompt": "." }
                ]
            }),
            &ctx(tmp.path()),
        )
        .await;
    let output: serde_json::Value = serde_json::from_str(&create_result.content).unwrap();
    let team_id = output["team_id"].as_str().unwrap().to_string();

    // Send a message
    let send_tool = SendMessageTeamTool::new(tmp.path().to_path_buf());
    let send_result = send_tool
        .execute(
            serde_json::json!({
                "team_id": team_id,
                "from": "sender",
                "to": "receiver",
                "message": "Hello receiver!",
                "message_type": "Chat"
            }),
            &ctx(tmp.path()),
        )
        .await;
    assert!(
        !send_result.is_error,
        "SendMessage must succeed: {}",
        send_result.content
    );

    // Read messages
    let read_tool = ReadTeamMessagesTool::new(tmp.path().to_path_buf());
    let read_result = read_tool
        .execute(
            serde_json::json!({
                "team_id": team_id,
                "role": "receiver"
            }),
            &ctx(tmp.path()),
        )
        .await;
    assert!(
        !read_result.is_error,
        "ReadTeamMessages must succeed: {}",
        read_result.content
    );
    let messages: serde_json::Value = serde_json::from_str(&read_result.content).unwrap();
    let msgs = messages["messages"].as_array().unwrap();
    assert_eq!(msgs.len(), 1);
    assert_eq!(msgs[0]["content"].as_str().unwrap(), "Hello receiver!");

    // Second read must return empty (cleared)
    let read_result2 = read_tool
        .execute(
            serde_json::json!({
                "team_id": team_id,
                "role": "receiver"
            }),
            &ctx(tmp.path()),
        )
        .await;
    let messages2: serde_json::Value = serde_json::from_str(&read_result2.content).unwrap();
    assert!(messages2["messages"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn send_to_all_delivers_to_all_members() {
    let tmp = tempfile::tempdir().unwrap();
    let create_tool = TeamCreateTool::new(tmp.path().to_path_buf());
    let create_result = create_tool
        .execute(
            serde_json::json!({
                "name": "broadcast-team",
                "members": [
                    { "role": "m1", "system_prompt": "." },
                    { "role": "m2", "system_prompt": "." },
                    { "role": "m3", "system_prompt": "." }
                ]
            }),
            &ctx(tmp.path()),
        )
        .await;
    let output: serde_json::Value = serde_json::from_str(&create_result.content).unwrap();
    let team_id = output["team_id"].as_str().unwrap().to_string();

    let send_tool = SendMessageTeamTool::new(tmp.path().to_path_buf());
    let send_result = send_tool
        .execute(
            serde_json::json!({
                "team_id": team_id,
                "from": "coordinator",
                "to": "all",
                "message": "Start your engines",
                "message_type": "TaskAssignment"
            }),
            &ctx(tmp.path()),
        )
        .await;
    assert!(!send_result.is_error);

    let read_tool = ReadTeamMessagesTool::new(tmp.path().to_path_buf());
    for role in ["m1", "m2", "m3"] {
        let result = read_tool
            .execute(
                serde_json::json!({
                    "team_id": team_id,
                    "role": role
                }),
                &ctx(tmp.path()),
            )
            .await;
        let data: serde_json::Value = serde_json::from_str(&result.content).unwrap();
        let msgs = data["messages"].as_array().unwrap();
        assert_eq!(
            msgs.len(),
            1,
            "role {} should have received 1 message",
            role
        );
    }
}

// ---------------------------------------------------------------------------
// TeamDelete
// ---------------------------------------------------------------------------

#[tokio::test]
async fn team_delete_removes_files() {
    let tmp = tempfile::tempdir().unwrap();
    let create_tool = TeamCreateTool::new(tmp.path().to_path_buf());
    let create_result = create_tool
        .execute(
            serde_json::json!({
                "name": "to-delete",
                "members": [{ "role": "x", "system_prompt": "." }]
            }),
            &ctx(tmp.path()),
        )
        .await;
    let output: serde_json::Value = serde_json::from_str(&create_result.content).unwrap();
    let team_id = output["team_id"].as_str().unwrap().to_string();

    let team_dir = tmp.path().join(".claude").join("teams").join(&team_id);
    assert!(team_dir.exists());

    let delete_tool = TeamDeleteTool::new(tmp.path().to_path_buf());
    let delete_result = delete_tool
        .execute(
            serde_json::json!({
                "team_id": team_id
            }),
            &ctx(tmp.path()),
        )
        .await;
    assert!(!delete_result.is_error);
    assert!(
        !team_dir.exists(),
        "team directory must be removed after delete"
    );
}
