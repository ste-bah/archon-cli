use super::*;

#[test]
fn directory_operands_and_uri_locations_resolve() {
    let f = Fixture::new("unused");
    let copy = f.copy();
    std::fs::create_dir_all(copy.repository().join("sub/nested")).unwrap();
    std::fs::write(copy.repository().join("sub/a"), b"source").unwrap();
    std::fs::write(copy.repository().join("sub/nested/b"), b"source").unwrap();
    for command in [
        "cd sub && go test",
        "check -C sub",
        "check --project=sub",
        "check --manifest-path sub/a",
    ] {
        assert_eq!(
            run::resolve(copy.copy(), command, copy.repository(), &[], "./a"),
            Some(("sub/a".into(), LocatedClass::CheckNamed)),
            "{command}"
        );
    }
    assert_eq!(
        run::resolve(
            copy.copy(),
            "cd sub && cd nested && check",
            copy.repository(),
            &[],
            "b"
        ),
        Some(("sub/nested/b".into(), LocatedClass::CheckNamed))
    );
    let path = copy
        .repository()
        .join("sub/a")
        .to_string_lossy()
        .replace('\\', "/");
    let uri = format!(
        "file://{}{}",
        if path.starts_with('/') { "" } else { "/" },
        path
    );
    assert_eq!(
        run::resolve(copy.copy(), "", copy.repository(), &["sub/a".into()], &uri),
        Some(("sub/a".into(), LocatedClass::Mutated))
    );
    assert_eq!(
        run::resolve(
            copy.copy(),
            "cd absent && check",
            copy.repository(),
            &[],
            "missing"
        ),
        None
    );
}

#[cfg(unix)]
#[tokio::test]
async fn time_cap_is_a_parameter_and_part_of_the_run_key() {
    let f = Fixture::new("test -f value");
    let mut p = f.probe(false);
    p.check_cap_secs = 1;
    let report = f.prove(&p, 10).await;
    let summary = report.proofs.summary.unwrap();
    assert!(summary.refusals.contains(&"invalid_time_cap".into()));
    assert_eq!(summary.runs_executed, 0);
    p.check_cap_secs = 2;
    let first = store::keys(&p, &f.contract, "check", "patch", 1)
        .await
        .unwrap();
    p.check_cap_secs = 3;
    let second = store::keys(&p, &f.contract, "check", "patch", 1)
        .await
        .unwrap();
    let third = store::keys(&p, &f.contract, "check", "patch", 2)
        .await
        .unwrap();
    assert_ne!(first.0, second.0);
    assert_ne!(first.1, second.1);
    assert_ne!(second.1, third.1);
}
