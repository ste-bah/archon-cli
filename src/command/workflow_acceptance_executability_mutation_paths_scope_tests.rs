use super::*;

fn fixture() -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "input-scope-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    std::fs::create_dir_all(root.join("archon-cli/nested")).unwrap();
    let binary = std::fs::File::create(root.join("archon")).unwrap();
    binary.set_len(100_173_120).unwrap();
    root
}

fn base_for(text: &str, root: &Path, needle: &str) -> Option<PathBuf> {
    let (commands, _) = lex(text);
    let (commands, bases, _) = effective_directories(&commands, root);
    commands
        .iter()
        .zip(bases)
        .find_map(|(words, base)| words.iter().any(|word| word == needle).then_some(base))
        .flatten()
}

#[test]
fn preceding_cd_applies_to_loop_substitution_and_heredoc() {
    let root = fixture();
    let text = r#"set -eu
cd archon-cli
for t in A B C D; do
  out=$(cargo test --bin archon --lib "$t" -- --exact 2>&1) || { printf '%s\n' "$out"; exit 1; }
  printf '%s\n' "$out" | grep -q "test result: ok" || { printf '%s\n' "$out"; exit 1; }
done
python3 - <<'PY'
from pathlib import Path
Path('crates/input.rs').read_text()
PY
"#;
    assert_eq!(
        base_for(text, &root, "archon"),
        Some(root.join("archon-cli"))
    );
    let (commands, bodies) = lex(text);
    let (_, bases, _) = effective_directories(&commands, &root);
    assert_eq!(bases[bodies[0].0], Some(root.join("archon-cli")));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn substitution_cd_does_not_escape_or_close_an_outer_subshell() {
    let root = fixture();
    for text in [
        "cd archon-cli; out=$(cd nested; inspect inner); inspect after",
        "cd archon-cli; out=\"$(cd nested; inspect inner)\"; inspect after",
        "(cd archon-cli; out=$(inspect inner); inspect after); inspect outside",
        "cd archon-cli; for t in A B; do out=$(cd nested; inspect inner); inspect after; done; inspect final",
    ] {
        assert_eq!(
            base_for(text, &root, "inner"),
            Some(root.join(if text.contains("cd nested") {
                "archon-cli/nested"
            } else {
                "archon-cli"
            })),
            "{text}"
        );
        assert_eq!(
            base_for(text, &root, "after"),
            Some(root.join("archon-cli")),
            "{text}"
        );
    }
    assert_eq!(
        base_for(
            "(cd archon-cli; inspect inside); inspect outside",
            &root,
            "outside"
        ),
        Some(root.clone())
    );
    assert_eq!(
        base_for(
            "cd archon-cli; out=$(cd nested); inspect after",
            &root,
            "after"
        ),
        Some(root.join("archon-cli"))
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn missing_directory_never_leaves_a_stale_base() {
    let root = fixture();
    for command in ["cd missing", "pushd missing"] {
        let text = format!("{command}; for t in A B; do out=$(inspect input); done; inspect after");
        assert_eq!(base_for(&text, &root, "input"), None, "{text}");
        assert_eq!(base_for(&text, &root, "after"), None, "{text}");
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn case_pattern_parentheses_do_not_close_shell_scopes() {
    let root = fixture();
    for text in [
        "cd archon-cli; out=$(case x in x) cd nested; inspect inner ;; esac; inspect after); inspect outside",
        "cd archon-cli; out=$(case esac in 'esac') cd nested; inspect inner ;; esac; inspect after); inspect outside",
        "cd archon-cli; out=$(case case in case|x) cd nested; inspect inner ;; esac; inspect after); inspect outside",
        "cd archon-cli; (case x in (x) cd nested; inspect inner ;; esac; inspect after); inspect outside",
        "cd archon-cli; out=\"$(case x in x|y) inspect first ;; *) cd nested; inspect inner ;; esac; inspect after)\"; inspect outside",
        "cd archon-cli; out=$(case x in x) case y in y) inspect first ;; esac; cd nested; inspect inner ;; esac; inspect after); inspect outside",
        "cd archon-cli; out=$(case x in x) value=$(inspect first); cd nested; inspect inner ;; esac; inspect after); inspect outside",
        "cd archon-cli; out=$(case \"$(uname)\" in Darwin) cd nested; inspect inner ;; esac; inspect after); inspect outside",
        "cd archon-cli; out=$(case $(uname) in Darwin) cd nested; inspect inner ;; esac; inspect after); inspect outside",
        "cd archon-cli; (case \"$(uname)\" in Darwin) cd nested; inspect inner ;; esac; inspect after); inspect outside",
        "cd archon-cli; (case $(uname) in Darwin) cd nested; inspect inner ;; esac; inspect after); inspect outside",
        "cd archon-cli; out=$(case x in x) inspect first ;& y) cd nested; inspect inner ;; esac; inspect after); inspect outside",
        "cd archon-cli; out=$(case x in x) inspect first ;;& y) cd nested; inspect inner ;; esac; inspect after); inspect outside",
        "cd archon-cli; (case x in x) inspect first ;& y) cd nested; inspect inner ;; esac; inspect after); inspect outside",
        "cd archon-cli; (case x in x) inspect first ;;& y) cd nested; inspect inner ;; esac; inspect after); inspect outside",
        "cd archon-cli; out=$(case \"$(uname)\" in Darwin) case $(uname) in Darwin) inspect first ;;& *) inspect second ;; esac; cd nested; inspect inner ;; esac; inspect after); inspect outside",
    ] {
        assert_eq!(
            base_for(text, &root, "inner"),
            Some(root.join("archon-cli/nested")),
            "{text}"
        );
        assert_eq!(
            base_for(text, &root, "after"),
            Some(root.join("archon-cli/nested")),
            "{text}"
        );
        assert_eq!(
            base_for(text, &root, "outside"),
            Some(root.join("archon-cli")),
            "{text}"
        );
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn exact_live_shape_with_missing_cd_target_has_no_input_base() {
    let root = fixture();
    std::fs::remove_dir_all(root.join("archon-cli")).unwrap();
    let text = r#"set -eu
cd archon-cli
for t in A B C D; do
  out=$(cargo test --bin archon --lib "$t" -- --exact 2>&1) || { printf '%s\n' "$out"; exit 1; }
  printf '%s\n' "$out" | grep -q "test result: ok" || { printf '%s\n' "$out"; exit 1; }
done
python3 - <<'PY'
from pathlib import Path
Path('crates/input.rs').read_text()
PY
"#;
    for binary in [false, true] {
        if binary {
            std::fs::File::create(root.join("archon"))
                .unwrap()
                .set_len(100_173_120)
                .unwrap();
        } else {
            std::fs::write(root.join("archon"), vec![b'x'; 300_000]).unwrap();
        }
        assert_eq!(base_for(text, &root, "archon"), None);
        let (commands, bodies) = lex(text);
        let (_, bases, unknown) = effective_directories(&commands, &root);
        assert!(unknown);
        assert_eq!(bases[bodies[0].0], None);
    }
    std::fs::remove_dir_all(root).unwrap();
}
