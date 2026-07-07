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
    let fake_home = tmp.path().join("home");

    // `default_memory_data_dir`/`default_config_path` resolve via `dirs::data_dir()`
    // / `dirs::config_dir()`, which derive from `$HOME` on every platform (and only
    // fall back to `$XDG_DATA_HOME`/`$XDG_CONFIG_HOME` on Linux) — so `HOME` is the
    // one override that isolates this smoke test from a machine's real durable
    // memory graph on both macOS and Linux.
    let output = Command::new(bin)
        .current_dir(tmp.path())
        .env("HOME", &fake_home)
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
