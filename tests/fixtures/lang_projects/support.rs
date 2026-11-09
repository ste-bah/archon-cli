// Shared test-only helpers. All scratch writes stay in this worktree.
#![allow(dead_code)]
use std::{
    path::{Path, PathBuf},
    process::{Command, Output},
};
pub fn root() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    while !p.join("tests/fixtures/lang_projects").is_dir() {
        assert!(p.pop());
    }
    p
}
pub fn fixture(lang: &str) -> PathBuf {
    root().join("tests/fixtures/lang_projects").join(lang)
}
pub fn scratch() -> tempfile::TempDir {
    let p = root().join("tests/agnostic-proof-results/tmp");
    std::fs::create_dir_all(&p).unwrap();
    tempfile::tempdir_in(p).unwrap()
}
pub fn copy_fixture(lang: &str) -> tempfile::TempDir {
    let dir = scratch();
    for entry in std::fs::read_dir(fixture(lang)).unwrap() {
        let path = entry.unwrap().path();
        if path.is_file() {
            std::fs::copy(&path, dir.path().join(path.file_name().unwrap())).unwrap();
        }
    }
    dir
}
pub fn available(tool: &str) -> bool {
    let ok = Command::new(tool)
        .arg(if tool == "go" { "version" } else { "--version" })
        .output()
        .is_ok_and(|o| o.status.success());
    assert!(ok, "toolchain missing: {tool}");
    ok
}
pub fn run(dir: &Path, tool: &str, args: &[&str]) -> Option<Output> {
    assert!(available(tool), "toolchain missing: {tool}");
    let mut cmd = Command::new(tool);
    cmd.args(args)
        .current_dir(dir)
        .env("PYTHONDONTWRITEBYTECODE", "1")
        .env("GOMAXPROCS", "2")
        .env("GOFLAGS", "-p=2")
        .env("GOPROXY", "off")
        .env("GOSUMDB", "off")
        .env("GOTOOLCHAIN", "local")
        .env("npm_config_offline", "true");
    let cache = root().join("tests/agnostic-proof-results/cache");
    std::fs::create_dir_all(&cache).unwrap();
    cmd.env("GOCACHE", cache.join("go"))
        .env("GOMODCACHE", cache.join("gomod"))
        .env("TMPDIR", &cache)
        .env("npm_config_cache", cache.join("npm"));
    Some(cmd.output().unwrap())
}
pub fn text(out: &Output) -> String {
    format!(
        "{}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}
pub fn failing_go() -> Option<String> {
    let out = run(
        &fixture("go"),
        "go",
        &["test", "-v", "-run", "^TestFailing$"],
    )?;
    assert!(!out.status.success(), "fixture's failing Go test must fail");
    Some(text(&out))
}
pub fn passing_python() -> Option<String> {
    let out = run(
        &fixture("python"),
        "python3",
        &["-m", "unittest", "-v", "test_pass"],
    )?;
    assert!(out.status.success(), "{}", text(&out));
    Some(text(&out))
}
pub fn zero_go() -> Option<String> {
    let out = run(&fixture("go"), "go", &["test", "-run", "^Missing$"])?;
    assert!(out.status.success(), "{}", text(&out));
    Some(text(&out))
}

/// A local executable fixture for each host platform.
pub fn version_stub(bin: &Path, name: &str, version: &str) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let path = bin.join(name);
        std::fs::write(&path, format!("#!/bin/sh\necho {version}\n")).unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    #[cfg(windows)]
    {
        std::fs::write(
            bin.join(format!("{name}.cmd")),
            format!("@echo off\r\necho {version}\r\n"),
        )
        .unwrap();
    }
}
