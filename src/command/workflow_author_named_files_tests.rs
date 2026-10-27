use super::*;

fn dispatch(ctx: &rquickjs::Ctx<'_>, paths: &str, cwd: &str, budget: usize) -> Value {
    let entry = serde_json::json!({
        "id": "AC-X-001", "criterion": "criterion",
        "check": {"kind": "command", "command": format!("operation {paths}"), "cwd": cwd}
    });
    let binding: rquickjs::Function = ctx.globals().get("__archonNamedFiles").unwrap();
    let result: String = binding.call((entry.to_string(), "[]", budget)).unwrap();
    serde_json::from_str(&result).unwrap()
}

fn with_binding(test: impl FnOnce(&rquickjs::Ctx<'_>, &std::path::Path)) {
    let root = tempfile::tempdir().unwrap();
    let runtime = rquickjs::Runtime::new().unwrap();
    let context = rquickjs::Context::full(&runtime).unwrap();
    context.with(|ctx| {
        install(
            &ctx,
            Some(&serde_json::json!({
                "projectRoot": root.path(),
                "repositoryRoot": env!("CARGO_MANIFEST_DIR"),
                "taskRoot": root.path()
            })),
        )
        .unwrap();
        test(&ctx, root.path());
    });
}

#[test]
fn named_files_dispatch_rejects_oversized_input_before_body_read() {
    with_binding(|ctx, root| {
        std::fs::write(root.join("input.data"), "abcd").unwrap();
        let before = judge::body_reads();
        for budget in [0, 3] {
            let files = dispatch(ctx, "input.data", "project_root", budget);
            assert_eq!(files[0]["status"], "not_supplied");
            assert_eq!(judge::body_reads(), before, "rejected input body was read");
            assert!(files[0]["detail"].as_str().unwrap().contains("byte limit"));
        }
        let files = dispatch(ctx, "input.data", "project_root", 4);
        assert_eq!(files[0]["content"], "abcd");
        assert_eq!(judge::body_reads(), before + 1);
    });
}

#[test]
fn named_files_dispatch_checks_remaining_budget_before_each_body_read() {
    with_binding(|ctx, root| {
        std::fs::write(root.join("first.data"), "abcd").unwrap();
        std::fs::write(root.join("second.data"), "efgh").unwrap();
        let before = judge::body_reads();
        let files = dispatch(ctx, "first.data second.data", "project_root", 4);
        assert_eq!(files[0]["content"], "abcd");
        assert_eq!(files[1]["status"], "not_supplied");
        assert_eq!(
            judge::body_reads(),
            before + 1,
            "rejected input body was read"
        );
    });
}

#[test]
fn named_files_dispatch_does_not_read_or_cache_rejected_blob() {
    with_binding(|ctx, _| {
        // Select an existing committed input without creating repository state.
        let output = archon_shell::spawn::command("git")
            .args([
                "-C",
                env!("CARGO_MANIFEST_DIR"),
                "ls-tree",
                "-r",
                "-l",
                "-z",
                "HEAD",
            ])
            .output()
            .unwrap();
        assert!(output.status.success());
        let (path, size) = output
            .stdout
            .split(|byte| *byte == 0)
            .find_map(|record| {
                let text = std::str::from_utf8(record).ok()?;
                let (metadata, path) = text.split_once('\t')?;
                let fields: Vec<_> = metadata.split_whitespace().collect();
                let size: usize = fields.get(3)?.parse().ok()?;
                (metadata.starts_with("100644 blob ") && path.starts_with("src/") && size > 1)
                    .then_some((path, size))
            })
            .expect("a committed regular input");
        let before = judge::body_reads();
        for budget in [0, size - 1] {
            let files = dispatch(ctx, path, "repo_root", budget);
            assert_eq!(files[0]["status"], "not_supplied");
            assert_eq!(judge::body_reads(), before, "rejected blob body was read");
        }
        let files = dispatch(ctx, path, "repo_root", size);
        assert_eq!(files[0]["status"], "supplied");
        assert_eq!(
            judge::body_reads(),
            before + 1,
            "rejected blob must not have been cached"
        );
        assert_eq!(dispatch(ctx, path, "repo_root", size), files);
        assert_eq!(judge::body_reads(), before + 1, "accepted blob is cached");
    });
}
