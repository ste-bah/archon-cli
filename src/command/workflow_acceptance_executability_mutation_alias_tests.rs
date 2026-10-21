use super::*;

#[cfg(unix)]
#[test]
fn repository_alias_input_moves_and_restores_at_its_canonical_copy_path() {
    let parent = tempfile::tempdir().unwrap();
    let project = parent.path().join("project");
    let copy_root = parent.path().join("copy/repository");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::create_dir_all(copy_root.join("tests")).unwrap();
    let named = "../repository-alias/focused.rs".to_string();
    let move_path = copy_root.join("tests/focused.rs");
    std::fs::write(&move_path, "focused").unwrap();
    std::os::unix::fs::symlink(
        &copy_root.join("tests"),
        parent.path().join("repository-alias"),
    )
    .unwrap();
    let markers = Markers::new();
    let script = super::wrapper_script::wrap_with_paths(
        "test -e ../repository-alias/focused.rs",
        &[named.clone()],
        &[move_path.to_string_lossy().into_owned()],
        &[Some(copy_root.to_string_lossy().into_owned())],
        &[],
        &markers,
    );
    let ran = sh(&project, &script);
    assert_eq!(ran.exit_code, Some(1));
    assert!(markers.restored_all(&ran));
    assert_eq!(std::fs::read_to_string(&move_path).unwrap(), "focused");
    assert!(String::from_utf8_lossy(&ran.stderr).contains("moved in the site copy"));
    assert!(markers.moved(&ran, &[named]).len() == 1);
}

#[cfg(unix)]
#[test]
fn swapping_the_repository_alias_during_the_check_cannot_redirect_restore() {
    let parent = tempfile::tempdir().unwrap();
    let project = parent.path().join("project");
    let copy_root = parent.path().join("copy/repository");
    let decoy = parent.path().join("decoy");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::create_dir_all(copy_root.join("tests")).unwrap();
    std::fs::create_dir_all(&decoy).unwrap();
    let named = "../repository-alias/focused.rs".to_string();
    let move_path = copy_root.join("tests/focused.rs");
    std::fs::write(&move_path, "copy original").unwrap();
    std::fs::write(decoy.join("focused.rs"), "decoy original").unwrap();
    let alias = parent.path().join("repository-alias");
    std::os::unix::fs::symlink(&copy_root.join("tests"), &alias).unwrap();
    let markers = Markers::new();
    let command = format!(
        "rm ../repository-alias; ln -s '{}' ../repository-alias; test -e ../repository-alias/focused.rs",
        decoy.display()
    );
    let script = super::wrapper_script::wrap_with_paths(
        &command,
        &[named],
        &[move_path.to_string_lossy().into_owned()],
        &[Some(copy_root.to_string_lossy().into_owned())],
        &[],
        &markers,
    );
    let ran = sh(&project, &script);
    assert_eq!(ran.exit_code, Some(0));
    assert!(markers.restored_all(&ran));
    assert_eq!(
        std::fs::read_to_string(&move_path).unwrap(),
        "copy original"
    );
    assert_eq!(
        std::fs::read_to_string(decoy.join("focused.rs")).unwrap(),
        "decoy original"
    );
}

#[cfg(unix)]
#[test]
fn a_symlink_planted_on_the_canonical_copy_path_is_refused() {
    let parent = tempfile::tempdir().unwrap();
    let project = parent.path().join("project");
    let copy_root = parent.path().join("copy/repository");
    let outside = parent.path().join("outside");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::create_dir_all(&copy_root).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(outside.join("focused.rs"), "outside").unwrap();
    std::os::unix::fs::symlink(&outside, copy_root.join("tests")).unwrap();
    let named = "../repository-alias/focused.rs".to_string();
    let move_path = copy_root.join("tests/focused.rs");
    let markers = Markers::new();
    let script = super::wrapper_script::wrap_with_paths(
        "false",
        &[named.clone()],
        &[move_path.to_string_lossy().into_owned()],
        &[Some(copy_root.to_string_lossy().into_owned())],
        &[],
        &markers,
    );
    let ran = sh(&project, &script);
    assert_eq!(ran.exit_code, Some(1));
    assert!(markers.moved(&ran, &[named.clone()]).is_empty());
    assert_eq!(
        markers
            .move_states(&ran, &[named])
            .values()
            .next()
            .map(String::as_str),
        Some("refused: symlink component")
    );
    assert_eq!(
        std::fs::read_to_string(outside.join("focused.rs")).unwrap(),
        "outside"
    );
}

#[cfg(unix)]
#[test]
fn replacing_the_copy_root_with_an_outside_symlink_refuses_move_and_restore() {
    let site = tempfile::tempdir().unwrap();
    let project = site.path().join("project");
    let copy_parent = site.path().join("copy");
    let copy_root = copy_parent.join("repository");
    let outside = site.path().join("decoy");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::create_dir_all(&copy_root).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(copy_root.join("focused.rs"), "copy").unwrap();
    std::fs::write(outside.join("focused.rs"), "decoy").unwrap();
    let named = "../repository-alias/focused.rs".to_string();
    let move_path = copy_root.join("focused.rs");
    let markers = Markers::new();
    let command = format!(
        "mv '{}' '{}.old'; rmdir '{}'; ln -s '{}' '{}'; true",
        copy_root.display(),
        copy_root.display(),
        copy_root.display(),
        outside.display(),
        copy_root.display()
    );
    let script = super::wrapper_script::wrap_with_paths(
        &command,
        &[named.clone()],
        &[move_path.to_string_lossy().into_owned()],
        &[Some(copy_root.to_string_lossy().into_owned())],
        &[],
        &markers,
    );
    let ran = sh(&project, &script);
    assert_eq!(markers.moved(&ran, &[named.clone()]).len(), 1);
    assert!(!markers.restored_all(&ran));
    assert_eq!(
        std::fs::read_to_string(outside.join("focused.rs")).unwrap(),
        "decoy"
    );
    assert!(String::from_utf8_lossy(&ran.stderr).contains("unsafe-to-restore"));
}

#[cfg(unix)]
#[test]
fn replacing_an_ancestor_of_the_copy_root_with_an_outside_symlink_is_refused() {
    let site = tempfile::tempdir().unwrap();
    let project = site.path().join("project");
    let copy_parent = site.path().join("copy");
    let copy_root = copy_parent.join("repository");
    let moved_parent = site.path().join("copy.old");
    let outside = site.path().join("decoy");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::create_dir_all(&copy_root).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(copy_root.join("focused.rs"), "copy").unwrap();
    std::fs::write(outside.join("focused.rs"), "decoy").unwrap();
    let named = "../repository-alias/focused.rs".to_string();
    let move_path = copy_root.join("focused.rs");
    let markers = Markers::new();
    let command = format!(
        "mv '{}' '{}'; ln -s '{}' '{}'; true",
        copy_parent.display(),
        moved_parent.display(),
        outside.display(),
        copy_parent.display()
    );
    let script = super::wrapper_script::wrap_with_paths(
        &command,
        &[named.clone()],
        &[move_path.to_string_lossy().into_owned()],
        &[Some(copy_root.to_string_lossy().into_owned())],
        &[],
        &markers,
    );
    let ran = sh(&project, &script);
    assert_eq!(markers.moved(&ran, &[named.clone()]).len(), 1);
    assert!(!markers.restored_all(&ran));
    assert_eq!(
        std::fs::read_to_string(outside.join("focused.rs")).unwrap(),
        "decoy"
    );
    assert!(String::from_utf8_lossy(&ran.stderr).contains("unsafe-to-restore"));
}
