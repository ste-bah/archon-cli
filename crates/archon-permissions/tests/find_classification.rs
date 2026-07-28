use archon_permissions::classifier::{CommandClass, classify_command};

fn no_overrides() -> (Vec<String>, Vec<String>, Vec<String>) {
    (vec![], vec![], vec![])
}

#[test]
fn find_delete_is_dangerous() {
    let (safe, risky, dangerous) = no_overrides();
    assert_eq!(
        classify_command("find /tmp/cache -type f -delete", &safe, &risky, &dangerous,),
        CommandClass::Dangerous
    );
}

#[test]
fn find_exec_rm_is_dangerous() {
    let (safe, risky, dangerous) = no_overrides();
    assert_eq!(
        classify_command(
            "find . -name '*.tmp' -exec rm -rf {} +",
            &safe,
            &risky,
            &dangerous,
        ),
        CommandClass::Dangerous
    );
}

#[test]
fn find_execdir_is_dangerous() {
    let (safe, risky, dangerous) = no_overrides();
    assert_eq!(
        classify_command(
            "find . -execdir touch marker {} +",
            &safe,
            &risky,
            &dangerous,
        ),
        CommandClass::Dangerous
    );
}

#[test]
fn absolute_path_find_destructive_predicates_are_dangerous() {
    let (safe, risky, dangerous) = no_overrides();
    for command in [
        "/usr/bin/find /tmp/cache -delete",
        "/usr/bin/find . -exec rm {} +",
        "/opt/bin/find . -execdir touch marker {} +",
    ] {
        assert_eq!(
            classify_command(command, &safe, &risky, &dangerous),
            CommandClass::Dangerous,
            "{command}"
        );
    }
}

#[test]
fn ordinary_absolute_path_find_keeps_unknown_command_fallback() {
    let (safe, risky, dangerous) = no_overrides();
    assert_eq!(
        classify_command(
            "/usr/bin/find . -name '*.rs' -print",
            &safe,
            &risky,
            &dangerous,
        ),
        CommandClass::Risky
    );
}

#[test]
fn find_destructive_predicates_with_attached_redirection_are_dangerous() {
    let (safe, risky, dangerous) = no_overrides();
    for command in [
        "find / -delete>/dev/null",
        "find / -delete&>/dev/null",
        "find / -exec>/dev/null rm {} +",
        "find / -execdir>/dev/null rm {} +",
    ] {
        assert_eq!(
            classify_command(command, &safe, &risky, &dangerous),
            CommandClass::Dangerous,
            "{command}"
        );
    }
}

#[test]
fn ordinary_find_remains_safe() {
    let (safe, risky, dangerous) = no_overrides();
    assert_eq!(
        classify_command("find . -name '*.rs' -print", &safe, &risky, &dangerous,),
        CommandClass::Safe
    );
}

#[test]
fn quoted_find_delete_predicate_is_dangerous() {
    let (safe, risky, dangerous) = no_overrides();
    assert_eq!(
        classify_command("find . '-delete'", &safe, &risky, &dangerous),
        CommandClass::Dangerous
    );
}

#[test]
fn find_name_path_and_regex_values_resembling_predicates_remain_safe() {
    let (safe, risky, dangerous) = no_overrides();
    for command in [
        "find . -name '-delete' -print",
        "find . -path '-exec' -print",
        "find . -regex '-execdir' -print",
    ] {
        assert_eq!(
            classify_command(command, &safe, &risky, &dangerous),
            CommandClass::Safe,
            "{command}"
        );
    }
}

#[test]
fn find_argument_values_resembling_predicates_remain_non_dangerous() {
    let (safe, risky, dangerous) = no_overrides();
    for command in [
        "find . -printf '-delete'",
        "/usr/bin/find . -fprintf report.txt '-exec'",
    ] {
        assert_ne!(
            classify_command(command, &safe, &risky, &dangerous),
            CommandClass::Dangerous,
            "{command}"
        );
    }
}

#[test]
fn windows_absolute_path_find_delete_is_dangerous() {
    let (safe, risky, dangerous) = no_overrides();
    assert_eq!(
        classify_command(r"C:\Tools\find . -delete", &safe, &risky, &dangerous,),
        CommandClass::Dangerous
    );
}

#[test]
fn user_safe_override_retains_priority_for_destructive_find() {
    let safe = vec!["find".to_string()];
    let (_, risky, dangerous) = no_overrides();
    assert_eq!(
        classify_command("find / -delete", &safe, &risky, &dangerous),
        CommandClass::Safe
    );
}
