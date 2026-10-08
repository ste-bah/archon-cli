use super::*;

#[test]
fn the_same_text_resolves_to_the_same_file_once() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path().join("run").join(AUTHOR_CONTEXT_DIR);
    let text = "{\"id\":\"AC-1\",\"check\":{\"command\":\"true\"}}";
    let (path, digest) = write_author_context(&dir, "json", text).unwrap();
    assert_eq!(digest, sha256_hex(text.as_bytes()));
    assert_eq!(path, dir.join(format!("{digest}.json")));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
    let again = write_author_context(&dir, "json", text).unwrap();
    assert_eq!(again, (path.clone(), digest));
    let names: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(names.len(), 1, "no staging file is left behind: {names:?}");
}

#[test]
fn a_file_that_holds_other_bytes_is_a_fault_and_is_kept() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path().to_path_buf();
    let (path, _) = write_author_context(&dir, "jsonl", "one\n").unwrap();
    std::fs::write(&path, "tampered\n").unwrap();
    let error = write_author_context(&dir, "jsonl", "one\n").unwrap_err();
    assert!(error.contains("holds other bytes than its name"), "{error}");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "tampered\n");
}

#[test]
fn multibyte_text_is_named_by_its_utf8_bytes() {
    let temp = tempfile::tempdir().unwrap();
    let text = "criterion: naïve — 東京 🚀";
    let (path, digest) = write_author_context(temp.path(), "txt", text).unwrap();
    assert_eq!(digest, sha256_hex(text.as_bytes()));
    assert_eq!(std::fs::read(&path).unwrap(), text.as_bytes());
}

#[test]
fn an_unknown_extension_and_an_unwritable_directory_are_refused() {
    let temp = tempfile::tempdir().unwrap();
    let error = write_author_context(temp.path(), "sh", "x").unwrap_err();
    assert!(error.contains("extension 'sh'"), "{error}");
    let blocker = temp.path().join("file");
    std::fs::write(&blocker, "not a directory").unwrap();
    let error = write_author_context(&blocker.join("dir"), "json", "x").unwrap_err();
    assert!(error.contains("author context"), "{error}");
}

#[test]
fn the_binding_returns_path_and_digest_and_a_preview_writes_nothing() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path().join(AUTHOR_CONTEXT_DIR);
    let runtime = rquickjs::Runtime::new().unwrap();
    let context = rquickjs::Context::full(&runtime).unwrap();
    context.with(|ctx| {
        install_author_context(&ctx, AuthorContextStore::Write(dir.clone())).unwrap();
        let out: String = ctx.eval(r#"__archonAuthorContext("json", "{\"a\":1}")"#).unwrap();
        let value: serde_json::Value = serde_json::from_str(&out).unwrap();
        let digest = sha256_hex(b"{\"a\":1}");
        assert_eq!(value["sha256"], digest.as_str());
        assert!(dir.join(format!("{digest}.json")).is_file());
        let thrown = ctx
            .eval::<String, _>(r#"(() => { try { __archonAuthorContext("exe", "x"); return "no"; } catch (e) { return String(e.message || e); } })()"#)
            .unwrap();
        assert!(thrown.contains("extension 'exe'"), "{thrown}");
    });
    let preview_dir = temp.path().join("preview");
    context.with(|ctx| {
        install_author_context(&ctx, AuthorContextStore::Preview).unwrap();
        let out: String = ctx.eval(r#"__archonAuthorContext("txt", "x")"#).unwrap();
        let value: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(
            value["path"],
            format!("{AUTHOR_CONTEXT_DIR}/{}.txt", sha256_hex(b"x"))
        );
    });
    assert!(!preview_dir.exists());
}
