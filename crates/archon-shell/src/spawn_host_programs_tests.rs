use super::*;
fn catalog(entries: &[(&str, &[&str])]) -> Catalog {
    Catalog {
        directories: entries
            .iter()
            .map(|(directory, names)| {
                (
                    PathBuf::from(directory),
                    names
                        .iter()
                        .map(|name| (key(OsStr::new(name)), OsString::from(name)))
                        .collect(),
                )
            })
            .collect(),
        extensions: vec![".com".into(), ".exe".into()],
        cache: Mutex::new(BTreeMap::new()),
    }
}
#[test]
fn directory_order_then_exact_name_then_extension_order() {
    let first = catalog(&[("first", &["tool.exe", "tool.com"]), ("second", &["tool"])]);
    assert_eq!(
        first.resolve(OsStr::new("tool"), |_| true),
        Some(Path::new("first").join("tool.com"))
    );
    let exact = catalog(&[("first", &["tool.exe", "tool.com", "tool"])]);
    assert_eq!(
        exact.resolve(OsStr::new("tool"), |_| true),
        Some(Path::new("first").join("tool"))
    );
}
#[test]
fn inspection_is_bounded_to_requested_candidates_and_cached() {
    let names: Vec<String> = (0..10000).map(|i| format!("unrelated{i}.exe")).collect();
    let refs: Vec<&str> = names
        .iter()
        .map(String::as_str)
        .chain(["tool.exe"])
        .collect();
    let catalog = catalog(&[("first", &refs)]);
    let mut inspected = 0;
    for _ in 0..100 {
        assert!(
            catalog
                .resolve(OsStr::new("tool"), |_| {
                    inspected += 1;
                    true
                })
                .is_some()
        );
        assert!(
            catalog
                .resolve(OsStr::new("missing"), |_| {
                    panic!("absent startup name inspected")
                })
                .is_none()
        );
    }
    assert_eq!(inspected, 1);
}
#[test]
fn executable_added_after_capture_is_never_inspected() {
    let root = tempfile::tempdir().unwrap();
    let catalog = catalog(&[(root.path().to_str().unwrap(), &[])]);
    std::fs::write(root.path().join("late.exe"), "planted").unwrap();
    assert!(
        catalog
            .resolve(OsStr::new("late"), |_| panic!("late entry inspected"))
            .is_none()
    );
}

#[test]
#[cfg(windows)]
fn validation_cannot_append_an_unrecorded_executable() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("tool"), "not executable").unwrap();
    let catalog = catalog(&[(root.path().to_str().unwrap(), &["tool"])]);
    std::fs::copy(
        std::env::current_exe().unwrap(),
        root.path().join("tool.exe"),
    )
    .unwrap();
    assert!(catalog.resolve(OsStr::new("tool"), candidate).is_none());
}

#[test]
#[cfg(windows)]
fn extensionless_binary_is_validated_by_its_exact_recorded_name() {
    let root = tempfile::tempdir().unwrap();
    let binary = root.path().join("tool");
    std::fs::copy(std::env::current_exe().unwrap(), &binary).unwrap();
    let catalog = catalog(&[(root.path().to_str().unwrap(), &["tool"])]);
    assert_eq!(catalog.resolve(OsStr::new("tool"), candidate), Some(binary));
}
