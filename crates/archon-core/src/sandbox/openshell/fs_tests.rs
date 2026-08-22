use super::*;

fn remote_config() -> OpenShellConfig {
    OpenShellConfig {
        enabled: true,
        workspace_mode: "remote".into(),
        gateway: Some("team-gateway".into()),
        remote_workdir: Some("/workspace/team".into()),
        ..OpenShellConfig::default()
    }
}

#[test]
fn mirror_mode_answers_from_the_host_because_that_is_the_working_tree() {
    let cfg = OpenShellConfig {
        workspace_mode: "mirror".into(),
        ..remote_config()
    };

    let fs = openshell_filesystem_for_mode(&cfg, Path::new("/home/dev/proj")).unwrap();

    assert!(format!("{fs:?}").contains("LocalFs"));
}

#[test]
fn upload_mode_answers_from_the_host_because_that_is_what_gets_uploaded() {
    // Each command re-uploads the host tree into a throwaway sandbox, so the
    // host tree is what the next command sees. Routing reads into the sandbox
    // would answer from a copy that is about to be destroyed.
    let cfg = OpenShellConfig {
        workspace_mode: "upload".into(),
        ..remote_config()
    };

    let fs = openshell_filesystem_for_mode(&cfg, Path::new("/home/dev/proj")).unwrap();

    assert!(format!("{fs:?}").contains("LocalFs"));
}

#[test]
fn remote_mode_answers_over_the_transport() {
    let fs = openshell_filesystem_for_mode(&remote_config(), Path::new("/home/dev/proj")).unwrap();

    let described = format!("{fs:?}");
    assert!(described.contains("RemoteFs"));
    assert!(described.contains("OpenShellTransport"));
}

#[test]
fn an_unknown_workspace_mode_is_refused() {
    let cfg = OpenShellConfig {
        workspace_mode: "sync".into(),
        ..remote_config()
    };

    assert!(openshell_filesystem_for_mode(&cfg, Path::new("/home/dev/proj")).is_err());
}

#[test]
fn a_relative_remote_workdir_is_refused_rather_than_resolved() {
    let cfg = OpenShellConfig {
        remote_workdir: Some("team".into()),
        ..remote_config()
    };

    let error = openshell_filesystem_for_mode(&cfg, Path::new("/home/dev/proj")).unwrap_err();

    assert!(error.contains("absolute path"));
}

#[test]
fn filesystem_commands_ride_the_same_sandbox_creation_as_bash() {
    let args = openshell_fs_args(
        &remote_config(),
        Path::new("/home/dev/proj"),
        "rm -- '/workspace/team/a.txt'\n",
    )
    .unwrap();

    assert_eq!(args[0], "sandbox");
    assert_eq!(args[1], "create");
    assert!(args.contains(&"--no-keep".to_string()));
    // remote mode never uploads: the tree is already on the far side.
    assert!(!args.contains(&"--upload".to_string()));
    assert!(args.contains(&"/bin/bash".to_string()));
    let script = args.last().unwrap();
    assert!(script.starts_with("cd -- '/workspace/team' && "));
    assert!(script.contains("rm -- '/workspace/team/a.txt'"));
}

#[test]
fn the_filesystem_roots_itself_where_bash_is_cd_ed_to() {
    let cfg = OpenShellConfig {
        remote_workdir: None,
        ..remote_config()
    };

    let args = openshell_fs_args(&cfg, Path::new("/home/dev/proj"), "true\n").unwrap();

    // Both sides fall back to the same default rather than to two guesses.
    assert!(args.last().unwrap().starts_with("cd -- '/sandbox' && "));
    assert_eq!(remote_workdir(&cfg), "/sandbox");
}

#[test]
fn a_config_the_backend_would_refuse_to_route_gets_no_filesystem() {
    let cfg = OpenShellConfig {
        provider_injection: true,
        ..remote_config()
    };

    let error = openshell_filesystem(&cfg, Path::new("/home/dev/proj")).unwrap_err();

    assert!(error.contains("provider injection"));
}
