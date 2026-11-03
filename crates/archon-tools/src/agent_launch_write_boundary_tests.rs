use super::*;

#[tokio::test]
async fn monitor_write_boundary_blocks_sealed_create_and_modify() {
    let layout = layout();
    let fresh = layout.project.join("monitor-created");
    let command = format!(
        "printf changed > '{}'; printf new > '{}'",
        layout.data.display(),
        fresh.display()
    );
    let result = crate::monitor::MonitorTool
        .execute(json!({"command": command}), &branch_ctx(&layout, true))
        .await;
    assert_eq!(
        read(&layout.data),
        "{\"shape\":\"v1\"}",
        "{}",
        result.content
    );
    assert!(
        !fresh.exists(),
        "Monitor must enforce the boundary or refuse launch: {}",
        result.content
    );
}

#[test]
fn monitor_host_grant_directory_is_write_protected_in_shared_profile() {
    let root = archon_shell::credential_paths::home().join(".archon/operator-env-grants");
    let rules = archon_shell::read_boundary::macos_rules(&[]);
    // The profile escapes '\' and '"' in paths (read_boundary::quoted), so a
    // Windows home path appears with doubled backslashes.
    let escaped = root
        .to_string_lossy()
        .replace('\\', "\\\\")
        .replace('"', "\\\"");
    assert!(
        rules.contains(&format!("(deny file-write* (subpath \"{escaped}\"))")),
        "host authority must be sealed after all writable allowances"
    );
}

#[cfg(unix)]
fn malicious_wrapper(layout: &Layout, name: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let script = layout.worktree.join(name);
    std::fs::write(
        &script,
        format!("#!/bin/sh\nprintf changed > '{}'\n", layout.data.display()),
    )
    .unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700)).unwrap();
    script
}

#[cfg(unix)]
#[tokio::test]
async fn java_write_boundary_blocks_agent_written_wrapper() {
    let layout = layout();
    std::fs::write(layout.worktree.join("build.gradle"), "").unwrap();
    malicious_wrapper(&layout, "gradlew");
    let result = crate::java::JavaToolchain
        .execute(json!({"operation": "compile"}), &branch_ctx(&layout, true))
        .await;
    assert_eq!(
        read(&layout.data),
        "{\"shape\":\"v1\"}",
        "{}",
        result.content
    );
}

#[cfg(unix)]
#[tokio::test]
async fn lsp_write_boundary_blocks_agent_selected_server() {
    let layout = layout();
    let script = malicious_wrapper(&layout, "language-server");
    let config = layout.worktree.join(".archon");
    std::fs::create_dir_all(&config).unwrap();
    std::fs::write(
        config.join("lsp-config.json"),
        serde_json::to_vec(&json!({"servers": [{"command": script}]})).unwrap(),
    )
    .unwrap();
    let tool = crate::lsp_tool::LspTool::new(Arc::new(tokio::sync::Mutex::new(
        crate::lsp_manager::LspServerManager::new(layout.worktree.clone(), None),
    )));
    let result = tool
        .execute(
            json!({"operation": "hover", "file_path": "src/lib.rs", "line": 0, "character": 0}),
            &branch_ctx(&layout, true),
        )
        .await;
    assert_eq!(
        read(&layout.data),
        "{\"shape\":\"v1\"}",
        "{}",
        result.content
    );
}

#[cfg(unix)]
#[tokio::test]
async fn monitor_write_boundary_protects_host_grant_records() {
    if std::env::var_os("ARCHON_R4_MONITOR_GRANT_PROBE").is_none() {
        let home = tempfile::tempdir().unwrap();
        let output = archon_shell::spawn::command(std::env::current_exe().unwrap())
            .args([
                "monitor_write_boundary_protects_host_grant_records",
                "--nocapture",
            ])
            .env("ARCHON_R4_MONITOR_GRANT_PROBE", "1")
            .env("HOME", home.path())
            .env("USERPROFILE", home.path())
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stdout)
        );
        return;
    }
    let layout = layout();
    let root = archon_shell::credential_paths::home().join(".archon/operator-env-grants");
    std::fs::create_dir_all(&root).unwrap();
    let record = root.join("test-run.json");
    let fresh = root.join("forged-run.json");
    std::fs::write(&record, "[]").unwrap();
    let command = format!(
        "printf changed > '{}'; printf forged > '{}'",
        record.display(),
        fresh.display()
    );
    let result = crate::monitor::MonitorTool
        .execute(json!({"command": command}), &branch_ctx(&layout, true))
        .await;
    assert_eq!(
        read(&record),
        "[]",
        "host grant record was modified: {}",
        result.content
    );
    assert!(
        !fresh.exists(),
        "host authority cannot be forged: {}",
        result.content
    );
}

#[test]
fn host_grant_record_is_not_a_file_tool_write_target() {
    let root = archon_shell::credential_paths::home();
    let ctx = ToolContext {
        working_dir: root.clone(),
        ..Default::default()
    };
    let target = root.join(".archon/operator-env-grants/forged-run.json");
    assert!(
        crate::path_guard::resolve_write_target_path(&target.to_string_lossy(), &ctx).is_err(),
        "host authority must be outside agent writable roots"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn terminal_write_boundary_blocks_agent_written_command() {
    let layout = layout();
    let ctx = branch_ctx(&layout, true);
    let launch = crate::terminal_world::plan(&ctx, Some("sh"), &layout.worktree);
    if let Ok(mut launch) = launch {
        launch.command.args([
            "-c",
            &format!("printf changed > '{}'", layout.data.display()),
        ]);
        let session = archon_pty::PtySession::spawn_agent_headless(
            launch.command,
            archon_pty::PtySize {
                rows: 24,
                cols: 80,
                pixel_width: 0,
                pixel_height: 0,
            },
        )
        .unwrap();
        let (_control, mut output) = session.split();
        while output.recv().await.is_some() {}
    }
    assert_eq!(read(&layout.data), "{\"shape\":\"v1\"}");
}
