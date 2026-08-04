use std::path::PathBuf;
use std::process::Command;

fn archon_bin() -> Option<PathBuf> {
    std::env::var_os("CARGO_BIN_EXE_archon").map(PathBuf::from)
}

#[test]
fn gnn_status_cli_smoke() {
    let Some(bin) = archon_bin() else {
        eprintln!("skipping: CARGO_BIN_EXE_archon not set");
        return;
    };
    let tmp = tempfile::tempdir().expect("tempdir");
    let config_home = tmp.path().join("config");
    let data_home = tmp.path().join("data");

    let output = Command::new(bin)
        .current_dir(tmp.path())
        .env("HOME", tmp.path())
        .env("XDG_CONFIG_HOME", &config_home)
        // XDG_DATA_HOME only redirects `dirs::data_dir()` on Linux -- Windows
        // reads the shell known-folder API and macOS ~/Library/Application
        // Support, so without this the child opens the real user database.
        .env("ARCHON_DATA_DIR", data_home.join("archon"))
        .env("XDG_DATA_HOME", &data_home)
        .args(["learning", "gnn", "status"])
        .output()
        .expect("run archon learning gnn status");

    assert!(
        output.status.success(),
        "status failed: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Enabled:           true"), "{stdout}");
    assert!(stdout.contains("First-run gate:    0/30"), "{stdout}");
    assert!(stdout.contains("New-memory gate:   0/20"), "{stdout}");
    assert!(stdout.contains("Correction gate:   0/3"), "{stdout}");
}
