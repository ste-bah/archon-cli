#[path = "../../build_fingerprint.rs"]
#[allow(dead_code)]
mod build_fingerprint;

fn fingerprint_root() -> tempfile::TempDir {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::create_dir_all(root.join("crates")).unwrap();
    for file in [
        "build.rs",
        "build_fingerprint.rs",
        "Cargo.toml",
        "Cargo.lock",
        "rust-toolchain.toml",
        ".cargo/config.toml",
    ] {
        let path = root.join(file);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, "input").unwrap();
    }
    temp
}

#[test]
fn reverify_fingerprint_changes_when_embedded_bytes_change() {
    let temp = fingerprint_root();
    let root = temp.path();
    std::fs::write(root.join("src/lib.rs"), "include_bytes!(\"asset.bin\");").unwrap();
    std::fs::write(root.join("src/asset.bin"), b"one").unwrap();

    let before = build_fingerprint::source_fingerprint(root);
    std::fs::write(root.join("src/asset.bin"), b"two").unwrap();

    assert_ne!(before, build_fingerprint::source_fingerprint(root));
}

#[test]
fn reverify_fingerprint_changes_when_compiler_flags_change() {
    let standard = build_fingerprint::combine_build_fingerprint("input", "flags-a", "profile");
    let changed = build_fingerprint::combine_build_fingerprint("input", "flags-b", "profile");

    assert_ne!(standard, changed);
}

#[test]
fn reverify_fingerprint_changes_when_profile_changes() {
    let standard = build_fingerprint::combine_build_fingerprint("input", "flags", "profile-a");
    let changed = build_fingerprint::combine_build_fingerprint("input", "flags", "profile-b");

    assert_ne!(standard, changed);
}

#[test]
fn reverify_fingerprint_is_stable_when_inputs_are_unchanged() {
    let temp = fingerprint_root();
    let root = temp.path();
    std::fs::write(root.join("src/lib.rs"), "include_bytes!(\"asset.bin\");").unwrap();
    std::fs::write(root.join("src/asset.bin"), b"stable").unwrap();

    let before = build_fingerprint::source_fingerprint(root);
    assert_eq!(before, build_fingerprint::source_fingerprint(root));
    assert_eq!(
        build_fingerprint::combine_build_fingerprint("input", "flags", "profile"),
        build_fingerprint::combine_build_fingerprint("input", "flags", "profile")
    );
}
