//! Tests for `archon config dump` (#189 Phase 7).

use super::*;

fn sample_config() -> ArchonConfig {
    ArchonConfig::default()
}

#[test]
fn the_header_names_the_file_and_whether_it_existed() {
    let path = std::path::Path::new("/home/u/.config/archon/config.toml");

    let loaded = render(path, &ConfigOrigin::Loaded, &sample_config(), &[]);
    assert!(
        loaded.contains("/home/u/.config/archon/config.toml"),
        "{loaded}"
    );
    assert!(loaded.contains("loaded from disk"), "{loaded}");

    let fresh = render(
        path,
        &ConfigOrigin::CreatedFromTemplate,
        &sample_config(),
        &[],
    );
    assert!(fresh.contains("defaults are in force"), "{fresh}");
}

/// "Which file, and did it exist" is half the answer to "why is it behaving
/// like this" — a missing file means defaults, not the file someone is reading.
#[test]
fn the_two_origins_read_differently() {
    let path = std::path::Path::new("/c.toml");
    assert_ne!(
        render(path, &ConfigOrigin::Loaded, &sample_config(), &[]),
        render(
            path,
            &ConfigOrigin::CreatedFromTemplate,
            &sample_config(),
            &[]
        )
    );
}

#[test]
fn the_resolved_config_is_rendered_as_toml() {
    let rendered = render(
        std::path::Path::new("/c.toml"),
        &ConfigOrigin::Loaded,
        &sample_config(),
        &[],
    );

    assert!(rendered.contains("## Resolved config"), "{rendered}");
    // Sections added by this issue must be visible, or the dump cannot explain
    // behaviour that depends on them.
    assert!(rendered.contains("[spill]"), "{rendered}");
    assert!(rendered.contains("[prune]"), "{rendered}");
}

#[test]
fn no_environment_says_so_rather_than_showing_an_empty_table() {
    let rendered = render(
        std::path::Path::new("/c.toml"),
        &ConfigOrigin::Loaded,
        &sample_config(),
        &[],
    );
    assert!(rendered.contains("(none set)"), "{rendered}");
}

#[test]
fn only_archon_variables_are_collected() {
    let entries = collect_env([
        ("ARCHON_SESSION_DB_PATH", "/tmp/s.db"),
        ("PATH", "/usr/bin"),
        ("HOME", "/home/u"),
    ]);

    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].name, "ARCHON_SESSION_DB_PATH");
}

#[test]
fn variables_are_listed_in_a_stable_order() {
    let entries = collect_env([
        ("ARCHON_Z_LAST", "1"),
        ("ARCHON_A_FIRST", "2"),
        ("ARCHON_M_MIDDLE", "3"),
    ]);

    let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(
        names,
        vec!["ARCHON_A_FIRST", "ARCHON_M_MIDDLE", "ARCHON_Z_LAST"]
    );
}

/// This output is going to be pasted into an issue.
#[test]
fn secret_shaped_values_are_redacted() {
    let entries = collect_env([("ARCHON_SOME_TOKEN", "sk-ant-api03_ZZZZZZZZZZZZZZZZZZ1234")]);

    assert!(
        !entries[0].value.contains("sk-ant-api03"),
        "raw secret survived: {}",
        entries[0].value
    );
    assert!(
        entries[0].value.contains("REDACTED"),
        "{}",
        entries[0].value
    );
}

/// A name set in the environment but read nowhere is almost always a typo, and
/// a typo here fails silently — the setting simply does nothing.
#[test]
fn an_unrecognised_variable_is_flagged() {
    // Assembled at runtime so the literal cannot appear in any source file for
    // the build-time scan to find — belt as well as braces, since the scan
    // already skips test modules.
    let fake = format!("ARCHON_{}_{}", "NOT_A_REAL", "SETTING_XYZZY");
    let entries = collect_env([(fake, "1".to_string())]);

    assert!(!entries[0].recognised);

    let rendered = render(
        std::path::Path::new("/c.toml"),
        &ConfigOrigin::Loaded,
        &sample_config(),
        &entries,
    );
    assert!(rendered.contains("<- unrecognised"), "{rendered}");
    assert!(rendered.contains("usually a typo"), "{rendered}");
}

/// The generated list is what makes the typo check trustworthy. If the scan
/// produced nothing, every real variable would be reported as a typo.
#[test]
fn the_generated_known_list_is_populated_and_well_formed() {
    assert!(
        KNOWN_ARCHON_ENV_VARS.len() > 50,
        "the build-time scan found only {} names, which cannot be right",
        KNOWN_ARCHON_ENV_VARS.len()
    );
    assert!(
        KNOWN_ARCHON_ENV_VARS
            .iter()
            .all(|name| name.starts_with("ARCHON_") && name.len() > "ARCHON_".len()),
        "every entry must be a full variable name"
    );
}

/// Variables this issue introduced or relies on must be in the generated set,
/// or `config dump` would report them as typos.
#[test]
fn variables_this_build_actually_reads_are_recognised() {
    for name in ["ARCHON_SESSION_DB_PATH", "ARCHON_EVIDENCE_DB_PATH"] {
        assert!(
            KNOWN_ARCHON_ENV_VARS.contains(&name),
            "{name} is read by this build but the scan missed it"
        );
    }
}

#[test]
fn a_recognised_variable_is_not_flagged() {
    let entries = collect_env([("ARCHON_SESSION_DB_PATH", "/tmp/s.db")]);

    assert!(entries[0].recognised);
    let rendered = render(
        std::path::Path::new("/c.toml"),
        &ConfigOrigin::Loaded,
        &sample_config(),
        &entries,
    );
    assert!(!rendered.contains("<- unrecognised"), "{rendered}");
}
