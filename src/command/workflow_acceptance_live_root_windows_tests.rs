#[cfg(windows)]
use super::*;

#[cfg(windows)]
#[test]
fn windows_forms_drop_every_filesystem_root_and_keep_both_spellings() {
    // No share is resolved: a UNC root is judged by its components alone.
    for root in [
        r"C:\",
        r"\\?\C:\",
        r"\\server\share\",
        r"\\server\share",
        "D:",
        r"\\?\D:",
    ] {
        assert!(!below_a_filesystem_root(Path::new(root)), "{root}");
    }
    assert_eq!(
        root_forms([r"C:\", r"\\?\C:\", "D:", "/"].map(Path::new)),
        Vec::<PathBuf>::new()
    );
    assert_eq!(
        verbatim(Path::new(r"\\server\share\x")),
        Some(PathBuf::from(r"\\?\UNC\server\share\x"))
    );
    let forms = root_forms([Path::new(r"C:\Work\Repo\")]);
    for form in [r"\\?\C:\Work\Repo", r"C:\Work\Repo"] {
        assert!(
            forms.iter().any(|f| f.as_os_str() == form),
            "{form}: {forms:?}"
        );
    }
    assert!(named_root(r"cd C:\Work\Repo-x\..\repo\src", &forms).is_some());
    assert!(named_root(r"cat C:\Work\\Repo\x", &forms).is_some());
    let live = tempfile::tempdir().unwrap();
    let canonical = live.path().canonicalize().unwrap();
    let forms = root_forms([live.path()]);
    for form in [canonical.clone(), archon_shell::paths::plain(canonical)] {
        assert!(forms.contains(&form), "{form:?}: {forms:?}");
    }
}

#[cfg(windows)]
#[test]
fn windows_short_name_alias_is_matched_by_file_identity() {
    use std::os::windows::ffi::{OsStrExt, OsStringExt};

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetShortPathNameW(input: *const u16, output: *mut u16, capacity: u32) -> u32;
    }

    let temp = match std::env::var_os("ARCHON_8DOT3_TEST_ROOT") {
        Some(root) => tempfile::Builder::new()
            .prefix("archon-short-name-")
            .tempdir_in(root)
            .expect("8.3 test root from CI must be writable"),
        None => tempfile::tempdir().unwrap(),
    };
    if let Some(configured_root) = std::env::var_os("ARCHON_8DOT3_TEST_ROOT") {
        let test_volume = temp.path().to_string_lossy()[..2].to_ascii_lowercase();
        let configured_volume = configured_root.to_string_lossy()[..2].to_ascii_lowercase();
        assert_eq!(
            test_volume, configured_volume,
            "short-name test directory must use the volume checked by Windows CI"
        );
    }
    let root = temp.path().join("Long Directory Name").join("Repository");
    std::fs::create_dir_all(&root).unwrap();
    let input: Vec<u16> = root.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut output = vec![0u16; 32_768];
    // SAFETY: both buffers are valid for the duration of the call, and the
    // output buffer's capacity is supplied to the Win32 API.
    let length =
        unsafe { GetShortPathNameW(input.as_ptr(), output.as_mut_ptr(), output.len() as u32) };
    assert!(
        length > 0 && (length as usize) < output.len(),
        "8.3 names are required for this Windows CI test; GetShortPathNameW failed"
    );
    output.truncate(length as usize);
    let short = PathBuf::from(OsString::from_wide(&output));
    assert_ne!(
        short, root,
        "8.3 names are required for this Windows CI test; the volume returned no short alias"
    );
    let forms = root_forms([root.as_path()]);
    assert!(named_root(&format!("type {}", short.display()), &forms).is_some());
    let unresolved_under_short_alias = short.join("not-created");
    assert!(
        named_root(
            &format!("type {}", unresolved_under_short_alias.display()),
            &forms
        )
        .is_some()
    );
    println!(
        "EVIDENCE #374 short-alias BEFORE: textual path differs from live root; AFTER: identity lookup refused alias={}",
        short.display()
    );
}

#[cfg(windows)]
#[test]
fn windows_unresolved_under_root_is_refused_and_sibling_is_allowed() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("Live Root");
    std::fs::create_dir(&root).unwrap();
    let forms = root_forms([root.as_path()]);
    let unresolved = root.join("unresolved").join("child");
    let sibling = temp.path().join("unrelated-missing").join("child");
    let live_volume = root
        .components()
        .next()
        .unwrap()
        .as_os_str()
        .to_string_lossy();
    let other_drive = if live_volume.to_lowercase().starts_with("c:") {
        "D:"
    } else {
        "C:"
    };
    let other_volume = PathBuf::from(format!(r"{other_drive}\unrelated-missing\child"));
    let unresolved_before = names_by_identity(&unresolved, &root);
    let unresolved_after =
        named_root(&format!("type \"{}\"", unresolved.display()), &forms).is_some();
    println!(
        "EVIDENCE #374 unresolved-under-root BEFORE: identity={} and path unresolved; AFTER: reconstructed live-root verdict={}",
        unresolved_before, unresolved_after
    );
    assert!(unresolved_after);

    let sibling_after = named_root(&format!("type \"{}\"", sibling.display()), &forms).is_some();
    println!(
        "EVIDENCE #374 unrelated-sibling BEFORE: path unresolved on live volume; AFTER: ancestor reconstruction verdict={}",
        sibling_after
    );
    assert!(!sibling_after);

    let other_volume_after =
        named_root(&format!("type \"{}\"", other_volume.display()), &forms).is_some();
    println!(
        "EVIDENCE #374 different-volume-control BEFORE: unresolved path on another volume; AFTER: fail-closed verdict={}",
        other_volume_after
    );
    assert!(!other_volume_after);
}
