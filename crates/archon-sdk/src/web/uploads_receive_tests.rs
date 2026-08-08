use super::*;

#[test]
fn traversal_segments_cannot_survive_sanitisation() {
    for raw in [
        "../../etc/passwd",
        "..\\..\\windows\\system32\\config\\sam",
        "/etc/shadow",
        "C:\\Windows\\notepad.exe",
        "....//....//x",
    ] {
        let name = safe_file_name(raw);
        assert!(!name.contains('/'), "{raw} -> {name}");
        assert!(!name.contains('\\'), "{raw} -> {name}");
        assert!(!name.contains(".."), "{raw} -> {name}");
        assert!(!name.starts_with('.'), "{raw} -> {name}");
    }
}

#[test]
fn joining_a_sanitised_name_stays_inside_the_directory() {
    let dir = Path::new("/staging/abc");
    for raw in ["../escape.txt", "..\\escape.txt", "ok.pdf"] {
        let joined = dir.join(safe_file_name(raw));
        assert_eq!(
            joined.parent(),
            Some(dir),
            "{raw} escaped to {}",
            joined.display()
        );
    }
}

#[test]
fn ordinary_names_are_left_recognisable() {
    assert_eq!(
        safe_file_name("Quarterly Report.pdf"),
        "Quarterly Report.pdf"
    );
    assert_eq!(safe_file_name("notes-2026_final.md"), "notes-2026_final.md");
    // Only the directory part is dropped, not the name itself.
    assert_eq!(safe_file_name("docs/design.md"), "design.md");
}

#[test]
fn a_name_that_sanitises_to_nothing_still_produces_a_file() {
    for raw in ["...", "/", "\\", "   ", ".."] {
        assert_eq!(safe_file_name(raw), "upload", "raw={raw:?}");
    }
}

#[test]
fn long_names_are_truncated_but_keep_the_extension() {
    let raw = format!("{}.pdf", "a".repeat(400));
    let name = safe_file_name(&raw);
    assert!(name.chars().count() <= MAX_NAME_LEN, "len={}", name.len());
    assert!(name.ends_with(".pdf"), "{name}");
}

#[test]
fn control_characters_and_shell_metacharacters_are_replaced() {
    // The stored path is typed into a terminal, so a name carrying an escape
    // sequence or a quote must not arrive intact.
    let name = safe_file_name("a\u{1b}[31mred\u{7};rm -rf ~\".txt");
    assert!(!name.contains('\u{1b}'), "{name}");
    assert!(!name.contains(';'), "{name}");
    assert!(!name.contains('"'), "{name}");
    assert!(name.ends_with(".txt"), "{name}");
}

#[test]
fn staging_root_is_under_archon_home() {
    let root = staging_root(Path::new("/home/user/.archon"));
    assert!(
        root.ends_with("web/uploads") || root.ends_with("web\\uploads"),
        "{root:?}"
    );
}
