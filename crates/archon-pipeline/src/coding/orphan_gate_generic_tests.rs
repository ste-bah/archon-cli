use super::orphan_gate::orphan_result;

#[test]
fn reference_fallback_covers_additional_languages_and_custom_extensions() {
    for extension in ["java", "kt", "cs", "swift", "custom"] {
        let tmp = tempfile::tempdir().unwrap();
        let new_file = write_source(
            tmp.path(),
            &format!("src/Widget.{extension}"),
            b"definition Widget\n",
        );
        write_source(
            tmp.path(),
            &format!("src/Main.{extension}"),
            b"use Widget\n",
        );
        let result = orphan_result(&[new_file], tmp.path(), "test".into());
        assert!(result.gate_passed, "{extension}: {}", result.evidence);
    }
}

fn write_source(root: &std::path::Path, relative: &str, bytes: &[u8]) -> std::path::PathBuf {
    let path = root.join(relative);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, bytes).unwrap();
    path
}
