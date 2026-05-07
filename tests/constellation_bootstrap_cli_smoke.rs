use std::path::PathBuf;
use std::process::Command;

fn archon_bin() -> Option<PathBuf> {
    std::env::var_os("CARGO_BIN_EXE_archon").map(PathBuf::from)
}

#[test]
fn constellation_bootstrap_cli_smoke() {
    let Some(bin) = archon_bin() else {
        eprintln!("skipping: CARGO_BIN_EXE_archon not set");
        return;
    };
    let tmp = tempfile::tempdir().expect("tempdir");
    let db_path = tmp.path().join("constellation.db");
    let fixture = tmp.path().join("representatives.txt");
    std::fs::write(
        &fixture,
        [
            "memory correction accepted after evidence review",
            "memory patch verified with regression coverage",
            "memory operator preference captured as durable rule",
            "memory source contradiction resolved with citation",
            "memory session summary retained for future work",
        ]
        .join("\n"),
    )
    .expect("write fixture");

    let output = Command::new(&bin)
        .current_dir(tmp.path())
        .env("ARCHON_CONSTELLATION_DB_PATH", &db_path)
        .args([
            "constellation",
            "bootstrap",
            "--target",
            "memory",
            "--inline-file",
            fixture.to_str().expect("fixture path"),
        ])
        .output()
        .expect("run archon constellation bootstrap");

    assert!(
        output.status.success(),
        "bootstrap failed: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Constellation bootstrap complete"),
        "{stdout}"
    );
    assert!(stdout.contains("Target: memory"), "{stdout}");
    assert!(stdout.contains("Samples used: 5"), "{stdout}");

    let list = Command::new(bin)
        .current_dir(tmp.path())
        .env("ARCHON_CONSTELLATION_DB_PATH", &db_path)
        .args(["constellation", "list"])
        .output()
        .expect("run archon constellation list");

    assert!(
        list.status.success(),
        "list failed: stdout={} stderr={}",
        String::from_utf8_lossy(&list.stdout),
        String::from_utf8_lossy(&list.stderr)
    );
    let list_stdout = String::from_utf8_lossy(&list.stdout);
    assert!(list_stdout.contains("target=memory"), "{list_stdout}");
    assert!(list_stdout.contains("1 centroids"), "{list_stdout}");
}
