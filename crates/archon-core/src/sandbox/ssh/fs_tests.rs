use super::*;

fn remote_config() -> SshConfig {
    SshConfig {
        enabled: true,
        host: Some("sandbox.example".into()),
        user: Some("archon".into()),
        port: 2222,
        remote_workdir: Some("/srv/workspace".into()),
        ..SshConfig::default()
    }
}

#[test]
fn mirror_mode_answers_from_the_host_because_that_is_the_working_tree() {
    let cfg = SshConfig {
        workspace_mode: "mirror".into(),
        ..remote_config()
    };

    let fs = ssh_filesystem_for_mode(&cfg, Path::new("/home/dev/proj")).unwrap();

    assert!(format!("{fs:?}").contains("LocalFs"));
}

#[test]
fn remote_mode_answers_over_the_transport() {
    let fs = ssh_filesystem_for_mode(&remote_config(), Path::new("/home/dev/proj")).unwrap();

    let described = format!("{fs:?}");
    assert!(described.contains("RemoteFs"));
    assert!(described.contains("SshTransport"));
}

#[test]
fn remote_mode_without_a_remote_workdir_has_no_filesystem_to_offer() {
    let cfg = SshConfig {
        remote_workdir: None,
        ..remote_config()
    };

    let error = ssh_filesystem_for_mode(&cfg, Path::new("/home/dev/proj")).unwrap_err();

    assert!(error.contains("remote_workdir"));
}

#[test]
fn a_relative_remote_workdir_is_refused_rather_than_resolved() {
    let cfg = SshConfig {
        remote_workdir: Some("workspace".into()),
        ..remote_config()
    };

    let error = ssh_filesystem_for_mode(&cfg, Path::new("/home/dev/proj")).unwrap_err();

    assert!(error.contains("absolute path"));
}

#[test]
fn an_unknown_workspace_mode_is_refused() {
    let cfg = SshConfig {
        workspace_mode: "sync".into(),
        ..remote_config()
    };

    assert!(ssh_filesystem_for_mode(&cfg, Path::new("/home/dev/proj")).is_err());
}

#[test]
fn filesystem_commands_ride_the_same_hardened_ssh_invocation_as_bash() {
    let args = ssh_fs_args(
        &remote_config(),
        Path::new("/home/dev/proj"),
        "stat -c '%s %Y %F' -- '/srv/workspace/a.txt'\n",
    )
    .unwrap();

    assert_eq!(args[0], "-T");
    assert!(args.contains(&"BatchMode=yes".to_string()));
    assert!(args.contains(&"StrictHostKeyChecking=yes".to_string()));
    assert!(args.contains(&"ForwardAgent=no".to_string()));
    assert!(args.contains(&"archon@sandbox.example".to_string()));
    // The script reaches the far side as one `bash -lc` argument, so its
    // quotes and newlines are the script's own and not the ssh command line's.
    let remote = args.last().unwrap();
    assert!(remote.starts_with("cd -- '/srv/workspace' && /bin/bash -lc "));
    assert!(remote.contains(r#"'\''%s %Y %F'\''"#));
}

#[test]
fn a_config_the_backend_would_refuse_to_route_gets_no_filesystem() {
    let cfg = SshConfig {
        host_key_checking: false,
        ..remote_config()
    };

    let error = ssh_filesystem(&cfg, Path::new("/home/dev/proj")).unwrap_err();

    assert!(error.contains("host-key checking"));
}
