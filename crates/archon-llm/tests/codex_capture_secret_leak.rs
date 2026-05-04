use std::path::Path;

const FIXTURE_ROOT: &str = "tests/fixtures/codex";

#[test]
fn codex_capture_fixtures_do_not_contain_live_secret_patterns() {
    let mut checked = 0usize;
    visit(Path::new(FIXTURE_ROOT), &mut |path| {
        let content = std::fs::read_to_string(path).unwrap_or_default();
        checked += 1;
        for forbidden in [
            "sk-",
            "Bearer live",
            "refresh-real",
            "access-real",
            "acct_real",
            "codex-refresh-token",
            "codex-access-token",
        ] {
            assert!(
                !content.contains(forbidden),
                "{} contains forbidden secret marker {forbidden}",
                path.display()
            );
        }
    });

    assert!(checked >= 8, "expected fixture files to be checked");
}

fn visit(dir: &Path, on_file: &mut impl FnMut(&Path)) {
    let entries = std::fs::read_dir(dir).unwrap_or_else(|_| std::process::exit(1));
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            visit(&path, on_file);
        } else {
            on_file(&path);
        }
    }
}
