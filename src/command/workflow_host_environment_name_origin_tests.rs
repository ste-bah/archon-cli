use super::*;

#[test]
fn host_environment_name_origin_literal_mutation() {
    for text in [
        r#"fn value() -> &'static str { "ARCHON_X_050" }"#,
        r#"const VALUE: &str = "ARCHON_X_050";"#,
        r#"#[value(name = "ARCHON_X_050")] fn value() {}"#,
        r#"const VALUE: &str = "\u{41}RCHON_X_050";"#,
        r#"const VALUE: &str = "\x41RCHON_X_050";"#,
        r#"const VALUE: &[u8] = b"ARCHON_X_050";"#,
        r##"const VALUE: &str = r#"ARCHON_X_050"#;"##,
        r#"fn value() { stringify!(ARCHON_X_050); }"#,
        r#"fn value() { let callback = |key| key; callback("ARCHON_X_050"); }"#,
        r#"fn value() { if let Some(key) = Some("ARCHON_X_050") {} }"#,
    ] {
        assert_eq!(
            warnings(owned_names(&[production_tokens(text)])),
            vec!["ARCHON_X_050"],
            "{text}"
        );
    }
}

#[test]
fn host_environment_name_origin_runtime_prefix_mutation() {
    for expression in [
        r#"format!("ARCHON_{}", x)"#,
        r#"format!("ARCHON_{x}")"#,
        r#"concat!("ARCHON_", x)"#,
        r#"{ let mut name = String::from("ARCHON_"); name.push_str(x); name }"#,
        r#""prefix ARCHON_""#,
        r#""prefix_ARCHON_""#,
    ] {
        let text = format!("fn value(x: &str) {{\n    {expression};\n}}");
        let failure = std::panic::catch_unwind(|| owned_names(&[production_tokens(&text)]))
            .expect_err("runtime-built name passed the guard");
        let message = failure.downcast_ref::<String>().unwrap();
        assert!(message.contains("fixture.rs:2"), "{message}");
        assert!(
            message.contains("runtime-built environment name"),
            "{message}"
        );
    }
    // Observing a prefix never constructs a name. Runtime-configured names
    // introduce no host-defined text, so they need no exception list.
    assert!(
        owned_names(&[production_tokens(
            r#"
        fn value(name: &str) {
            name.starts_with("ARCHON_"); name.find("ARCHON_");
            let length = "ARCHON_".len(); command.env(name, "1");
        }
    "#
        )])
        .is_empty()
    );
}

#[test]
fn host_environment_name_origin_concat_mutation() {
    for expression in [
        r#"concat!("ARCHON_", "X_051")"#,
        r#"concat! { "ARCHON_", "X_051" }"#,
        r#"concat!(concat!("ARCHON_", "X_"), "051")"#,
        r#"format!("ARCHON_{}", "X_051")"#,
        r#"format!("{}{}", "ARCHON_", "X_051")"#,
    ] {
        let text = format!("fn value() {{ let value = {expression}; }}");
        assert_eq!(
            warnings(owned_names(&[production_tokens(&text)])),
            vec!["ARCHON_X_051"]
        );
    }
}

#[test]
fn host_environment_name_origin_test_module_mutation() {
    let sources = [production_tokens(
        r#"
        #[cfg(test)]
        mod checks { const VALUE: &str = "ARCHON_X_052"; }
        #[cfg(windows)]
        fn value() -> &'static str { "ARCHON_X_053" }
    "#,
    )];
    assert_eq!(warnings(owned_names(&sources)), vec!["ARCHON_X_053"]);

    let temp = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(temp.path().join("src")).unwrap();
    std::fs::create_dir_all(temp.path().join("crates")).unwrap();
    std::fs::write(
        temp.path().join("src/main.rs"),
        r#"
        #[cfg(test)]
        #[path = "support.rs"]
        mod checks;
        #[cfg_attr(not(test), allow(dead_code))]
        fn value() -> &'static str { "ARCHON_X_053" }
    "#,
    )
    .unwrap();
    std::fs::write(
        temp.path().join("src/support.rs"),
        r#"const VALUE: &str = "ARCHON_X_052";"#,
    )
    .unwrap();
    // This file has no module declaration. Discovery still inspects it.
    std::fs::write(
        temp.path().join("src/unused.rs"),
        r#"const VALUE: &str = "ARCHON_X_053";"#,
    )
    .unwrap();
    let sources = production_sources(temp.path());
    assert!(
        sources
            .iter()
            .any(|source| source.path.ends_with("unused.rs"))
    );
    assert!(
        !sources
            .iter()
            .any(|source| source.path.ends_with("support.rs"))
    );
    assert_eq!(warnings(owned_names(&sources)), vec!["ARCHON_X_053"]);
}

#[test]
fn host_environment_name_origin_nested_source_mutation() {
    let temp = tempfile::tempdir().unwrap();
    for directory in ["src", "crates", "aux"] {
        std::fs::create_dir_all(temp.path().join(directory)).unwrap();
    }
    std::fs::write(
        temp.path().join("src/main.rs"),
        r#"#[path = "../aux/names.rs"] mod values; fn main() {}"#,
    )
    .unwrap();
    std::fs::write(
        temp.path().join("aux/names.rs"),
        r#"pub const VALUE: &str = "ARCHON_X_054";"#,
    )
    .unwrap();
    assert_eq!(
        warnings(owned_names(&production_sources(temp.path()))),
        vec!["ARCHON_X_054"]
    );
}

#[test]
fn host_environment_name_origin_stringify_nesting_mutation() {
    for expression in [
        "stringify!(value(ARCHON_API_KEY) ARCHON_X_050)",
        "stringify!{value{ARCHON_API_KEY} ARCHON_X_050}",
        "stringify![value[ARCHON_API_KEY] ARCHON_X_050]",
    ] {
        let text = format!("fn value() {{ let value = {expression}; }}");
        assert_eq!(
            warnings(owned_names(&[production_tokens(&text)])),
            vec!["ARCHON_X_050"],
            "{expression}"
        );
    }
}

#[test]
fn host_environment_name_origin_attribute_argument_mutation() {
    for text in [
        r#"#[value(test)] fn value() -> &'static str { "ARCHON_X_050" }"#,
        r#"#![value(test)] const VALUE: &str = "ARCHON_X_050";"#,
    ] {
        assert_eq!(
            warnings(owned_names(&[production_tokens(text)])),
            vec!["ARCHON_X_050"]
        );
    }
}

#[test]
fn host_environment_name_origin_inner_test_sibling_mutation() {
    let text = r#"
        #![allow(dead_code)]
        #[cfg(test)]
        mod other_checks {}
        mod checks {
            #![cfg(test)]
            const VALUE: &str = "ARCHON_X_059";
        }
        fn value() -> &'static str { "ARCHON_X_060" }
    "#;
    assert_eq!(
        warnings(owned_names(&[production_tokens(text)])),
        vec!["ARCHON_X_060"]
    );
    let temp = tempfile::tempdir().unwrap();
    for directory in ["src/checks", "crates"] {
        std::fs::create_dir_all(temp.path().join(directory)).unwrap();
    }
    std::fs::write(
        temp.path().join("src/main.rs"),
        text.replace(
            "const VALUE: &str = \"ARCHON_X_059\";",
            "#[path = \"../support.rs\"] mod support;",
        ),
    )
    .unwrap();
    std::fs::write(
        temp.path().join("src/support.rs"),
        "const VALUE: &str = \"ARCHON_X_059\";",
    )
    .unwrap();
    assert_eq!(
        warnings(owned_names(&production_sources(temp.path()))),
        vec!["ARCHON_X_060"]
    );
}

#[test]
fn host_environment_name_origin_embedded_resource_mutation() {
    let temp = tempfile::tempdir().unwrap();
    for directory in ["src", "crates"] {
        std::fs::create_dir_all(temp.path().join(directory)).unwrap();
    }
    std::fs::write(temp.path().join("src/key.txt"), "\"ARCHON_X_062\"\n").unwrap();
    for invocation in [
        r#"include_str!("key.txt")"#,
        r#"include_bytes!("key.txt")"#,
        r#"include!("key.txt")"#,
    ] {
        std::fs::write(
            temp.path().join("src/main.rs"),
            format!("fn value() {{ let value = {invocation}; }}"),
        )
        .unwrap();
        assert_eq!(
            warnings(owned_names(&production_sources(temp.path()))),
            vec!["ARCHON_X_062"],
            "{invocation}"
        );
    }
}

#[test]
fn host_environment_name_origin_unresolved_resource_mutation() {
    let temp = tempfile::tempdir().unwrap();
    for directory in ["src", "crates"] {
        std::fs::create_dir_all(temp.path().join(directory)).unwrap();
    }
    std::fs::write(
        temp.path().join("src/main.rs"),
        "fn value() {\n    include_str!(computed!());\n}",
    )
    .unwrap();
    let failure = std::panic::catch_unwind(|| production_sources(temp.path()))
        .err()
        .expect("unresolved embedded resource passed the guard");
    let message = failure.downcast_ref::<String>().unwrap();
    assert!(message.contains("src/main.rs:2"), "{message}");
    assert!(
        message.contains("unresolved embedded resource"),
        "{message}"
    );
}

#[test]
fn host_environment_name_origin_generated_resource_mutation() {
    let temp = tempfile::tempdir().unwrap();
    for directory in ["src", "crates", "target"] {
        std::fs::create_dir_all(temp.path().join(directory)).unwrap();
    }
    std::fs::write(temp.path().join("target/key.txt"), "ARCHON_X_070").unwrap();
    for kind in ["include", "include_str", "include_bytes"] {
        std::fs::write(
            temp.path().join("src/main.rs"),
            format!(r#"fn value() {{ {kind}!(concat!(env!("OUT_DIR"), "/key.txt")); }}"#),
        )
        .unwrap();
        assert!(owned_names(&production_sources(temp.path())).is_empty());
    }
    for kind in ["include", "include_str", "include_bytes"] {
        std::fs::write(
            temp.path().join("src/main.rs"),
            format!(r#"fn value() {{ {kind}!("../target/key.txt"); }}"#),
        )
        .unwrap();
        assert!(owned_names(&production_sources(temp.path())).is_empty());
    }
}

#[test]
fn host_environment_name_origin_conservative_shell_resource_mutation() {
    let temp = tempfile::tempdir().unwrap();
    for directory in ["src", "crates"] {
        std::fs::create_dir_all(temp.path().join(directory)).unwrap();
    }
    std::fs::write(
        temp.path().join("src/main.rs"),
        r#"const VALUE: &str = include_str!("value.sh");"#,
    )
    .unwrap();
    for (script, unknown) in [
        ("ARCHON_X_071=1\necho \"$ARCHON_X_071\"\n", false),
        ("export ARCHON_X_071=1\n", true),
        ("ARCHON_X_071=1\nexport ARCHON_X_071\n", true),
        ("ARCHON_X_071=1 value\n", true),
        ("env ARCHON_X_071=1 value\n", true),
        ("echo \"${ARCHON_X_071}\"\n", true),
        ("echo \"$ARCHON_X_071\"\nARCHON_X_071=1\n", false),
        ("ARCHON_X_071=1; export ARCHON_X_071\n", true),
        ("ARCHON_X_071=1 OTHER=2 value\n", true),
        ("env -i ARCHON_X_071=1 value\n", true),
        ("local ARCHON_X_071=1; echo \"$ARCHON_X_071\"\n", true),
        ("echo '$ARCHON_X_071'\n", true),
        ("echo \\$ARCHON_X_071\n", true),
        ("# export ARCHON_X_071=1\necho 'ARCHON_X_071'\n", true),
    ] {
        std::fs::write(temp.path().join("src/value.sh"), script).unwrap();
        let sources = production_sources(temp.path());
        assert_eq!(
            warnings(owned_names(&sources)),
            if unknown {
                vec!["ARCHON_X_071"]
            } else {
                vec![]
            },
            "{script}"
        );
        if unknown {
            assert!(
                owned_name_locations(&sources)["ARCHON_X_071"]
                    .iter()
                    .all(|location| location.contains("value.sh:"))
            );
        }
    }
}

fn shell_export_mutation(script: &str, name: &str) {
    let temp = tempfile::tempdir().unwrap();
    for directory in ["src", "crates"] {
        std::fs::create_dir_all(temp.path().join(directory)).unwrap();
    }
    std::fs::write(
        temp.path().join("src/main.rs"),
        r#"const VALUE: &str = include_str!("value.sh");"#,
    )
    .unwrap();
    let resource = temp.path().join("src/value.sh");
    // A plain assignment and its reads remain local under the conservative rule.
    std::fs::write(&resource, format!("{name}=1\necho \"${name}\"\n")).unwrap();
    assert!(warnings(owned_names(&production_sources(temp.path()))).is_empty());
    std::fs::write(&resource, script).unwrap();
    let sources = production_sources(temp.path());
    assert_eq!(warnings(owned_names(&sources)), vec![name], "{script}");
    assert!(
        owned_name_locations(&sources)[name]
            .iter()
            .all(|location| location.contains("value.sh:"))
    );
}

#[test]
fn host_environment_shell_continued_set_mutation() {
    shell_export_mutation(
        "set \\\n -a\nARCHON_X_080=1\n/usr/bin/printenv ARCHON_X_080\n",
        "ARCHON_X_080",
    );
}

#[test]
fn host_environment_shell_any_set_option_mutation() {
    for option in ["-e", "+a", "-o nounset"] {
        shell_export_mutation(
            &format!("set {option}\nARCHON_X_081=1\necho \"$ARCHON_X_081\"\n"),
            "ARCHON_X_081",
        );
    }
}

#[test]
fn host_environment_shell_quoted_export_mutation() {
    shell_export_mutation("export \"ARCHON_X_072=1\"\n", "ARCHON_X_072");
}

#[test]
fn host_environment_shell_declare_export_mutation() {
    shell_export_mutation("declare -x ARCHON_X_073=1\n", "ARCHON_X_073");
}

#[test]
fn host_environment_shell_exec_env_mutation() {
    shell_export_mutation("exec env ARCHON_X_074=1 value\n", "ARCHON_X_074");
}

#[test]
fn host_environment_shell_allexport_mutation() {
    shell_export_mutation("set -a; ARCHON_X_075=1; value\n", "ARCHON_X_075");
}

#[test]
fn host_environment_shell_quoted_env_mutation() {
    shell_export_mutation("env \"ARCHON_X_076=1\" value\n", "ARCHON_X_076");
}

#[test]
fn host_environment_shell_conservative_words_mutation() {
    for command in [
        "export OTHER",
        "declare OTHER",
        "typeset OTHER",
        "readonly OTHER",
        "local OTHER",
        "env value",
        "set -a",
        "set -o allexport",
        "set -eau",
        "set -eu -a",
        "set -o nounset -o allexport",
        "\"export\" OTHER",
    ] {
        shell_export_mutation(
            &format!("ARCHON_X_078=1\n{command}\necho \"$ARCHON_X_078\"\n"),
            "ARCHON_X_078",
        );
    }
    shell_export_mutation(
        "cat <<'END'\nvalue.environ[\"ARCHON_X_079\"]=\"1\"\nEND\n",
        "ARCHON_X_079",
    );
    shell_export_mutation(
        "ARCHON_X_079=1\ncat <<'END'\nexport OTHER\nEND\nexport OTHER\n",
        "ARCHON_X_079",
    );
    for script in ["\"if\" ARCHON_X_079=1\n", "\"for\" ARCHON_X_079 in 1\n"] {
        shell_export_mutation(script, "ARCHON_X_079");
    }
    let temp = tempfile::tempdir().unwrap();
    for directory in ["src", "crates"] {
        std::fs::create_dir_all(temp.path().join(directory)).unwrap();
    }
    std::fs::write(
        temp.path().join("src/main.rs"),
        r#"const VALUE: &str = include_str!("value.sh");"#,
    )
    .unwrap();
    for script in [
        "ARCHON_X_079=1\n# export declare typeset readonly local env set -a\necho \"$ARCHON_X_079\"\n",
        "ARCHON_X_079=1\ncat <<'END'\nexport declare typeset readonly local env set -a\nEND\n",
        "ARCHON_X_079=1\ncat <<-\"END\"\n\tenv value\n\tEND\n",
        "ARCHON_X_079=1\necho \"<<'END'\"\n# export OTHER\n",
    ] {
        std::fs::write(temp.path().join("src/value.sh"), script).unwrap();
        assert!(
            warnings(owned_names(&production_sources(temp.path()))).is_empty(),
            "{script}"
        );
    }
}
