//! Tests for the #200 Phase 3 preset table and coherence check.

use super::*;

fn config_with(permission_mode: &str, backend: &str) -> ArchonConfig {
    let mut config = ArchonConfig::default();
    config.permissions.mode = permission_mode.to_string();
    config.sandbox.backend = backend.to_string();
    config
}

#[test]
fn every_preset_round_trips_through_apply_and_derive() {
    for preset in PERMISSION_PRESETS {
        let mut config = ArchonConfig::default();
        let applied = apply_permission_preset(&mut config, preset.name).expect("preset applies");

        assert_eq!(applied.name, preset.name);
        assert_eq!(
            derive_permission_preset(&config),
            preset.name,
            "reading back a config set from {} must name it, not {CUSTOM_PRESET}",
            preset.name
        );
    }
}

#[test]
fn applying_a_preset_writes_both_subsystems() {
    let mut config = ArchonConfig::default();
    apply_permission_preset(&mut config, "sandboxed-throwaway").expect("preset applies");

    assert_eq!(config.permissions.mode, "bubble");
    assert_eq!(config.sandbox.backend, "docker");
    assert_eq!(config.sandbox.mode, "all");
    assert_eq!(config.sandbox.scope, "turn");
    assert_eq!(config.sandbox.workspace_access, "scratch");
}

#[test]
fn applying_a_preset_touches_nothing_but_the_five_knobs() {
    // The preset layer records intent. Anything else it wrote would be a
    // second path into behaviour the checker and the backends already own.
    let mut config = ArchonConfig::default();
    config.permissions.allow_paths = vec!["/srv".into()];
    config.permissions.always_deny = vec![archon_permissions::rules::ToolRule {
        tool: "Bash".into(),
        pattern: "rm:*".into(),
    }];
    let before = config.clone();

    apply_permission_preset(&mut config, "sandboxed").expect("preset applies");

    assert_eq!(
        config.permissions.allow_paths,
        before.permissions.allow_paths
    );
    // `ToolRule` has no `PartialEq`, so compare the fields the rule is made of.
    let rule_shape = |rules: &[archon_permissions::rules::ToolRule]| {
        rules
            .iter()
            .map(|rule| (rule.tool.clone(), rule.pattern.clone()))
            .collect::<Vec<_>>()
    };
    assert_eq!(
        rule_shape(&config.permissions.always_deny),
        rule_shape(&before.permissions.always_deny)
    );
    assert_eq!(config.sandbox.docker, before.sandbox.docker);
    assert_eq!(config.sandbox.ssh, before.sandbox.ssh);
    assert_eq!(config.sandbox.openshell, before.sandbox.openshell);
}

#[test]
fn hand_editing_one_field_reads_back_as_custom() {
    let mut config = ArchonConfig::default();
    apply_permission_preset(&mut config, "sandboxed").expect("preset applies");
    assert_eq!(derive_permission_preset(&config), "sandboxed");

    // One knob moved by hand, the other four still the preset's.
    config.sandbox.scope = "tool".to_string();

    assert_eq!(
        derive_permission_preset(&config),
        CUSTOM_PRESET,
        "sandbox.scope was hand-edited away from the sandboxed preset, so no preset describes \
         this config any more"
    );
}

#[test]
fn hand_editing_the_permission_mode_alone_reads_back_as_custom() {
    let mut config = ArchonConfig::default();
    apply_permission_preset(&mut config, "read-only").expect("preset applies");
    config.permissions.mode = "dontAsk".to_string();

    assert_eq!(derive_permission_preset(&config), CUSTOM_PRESET);
}

#[test]
fn legacy_mode_aliases_do_not_read_back_as_custom() {
    let mut config = ArchonConfig::default();
    apply_permission_preset(&mut config, "unrestricted").expect("preset applies");
    // "yolo" is the legacy spelling of bypassPermissions and means the same
    // thing to the checker; it must mean the same thing here.
    config.permissions.mode = "yolo".to_string();

    assert_eq!(derive_permission_preset(&config), "unrestricted");
}

#[test]
fn stock_defaults_are_custom_not_a_preset() {
    // A fresh config is `default` + `disabled`, which is deliberately not one
    // of the five. Claiming it were a preset would misreport what is in force.
    assert_eq!(
        derive_permission_preset(&ArchonConfig::default()),
        CUSTOM_PRESET
    );
}

#[test]
fn unknown_preset_names_the_known_ones() {
    let mut config = ArchonConfig::default();
    let error = apply_permission_preset(&mut config, "read_only").unwrap_err();
    let message = error.to_string();

    assert!(message.contains("read_only"), "{message}");
    assert!(message.contains("read-only"), "{message}");
    assert!(message.contains("sandboxed-throwaway"), "{message}");
}

#[test]
fn default_preset_is_in_the_table() {
    assert!(find_permission_preset(DEFAULT_PRESET).is_some());
    assert!(find_permission_preset(CUSTOM_PRESET).is_none());
}

#[test]
fn preset_names_are_unique() {
    for (index, preset) in PERMISSION_PRESETS.iter().enumerate() {
        assert!(
            !PERMISSION_PRESETS[..index]
                .iter()
                .any(|earlier| earlier.name == preset.name),
            "duplicate preset name {}",
            preset.name
        );
    }
}

#[test]
fn every_preset_is_itself_coherent_and_valid() {
    for preset in PERMISSION_PRESETS {
        let mut config = ArchonConfig::default();
        apply_permission_preset(&mut config, preset.name).expect("preset applies");

        super::super::validate(&config).unwrap_or_else(|error| {
            panic!("preset {} does not validate: {error}", preset.name);
        });
        assert!(
            permission_coherence_warnings(&config).is_empty(),
            "preset {} is itself incoherent: {:?}",
            preset.name,
            permission_coherence_warnings(&config)
        );
    }
}

#[test]
fn inert_sandbox_fields_warn_and_name_both_fields() {
    let mut config = config_with("default", "disabled");
    config.sandbox.workspace_access = "scratch".to_string();
    config.sandbox.scope = "tool".to_string();

    let warnings = permission_coherence_warnings(&config);

    assert_eq!(warnings.len(), 2, "{warnings:?}");
    let joined = warnings.join("\n");
    assert!(joined.contains("sandbox.workspace_access"), "{joined}");
    assert!(joined.contains("sandbox.scope"), "{joined}");
    assert!(joined.contains("sandbox.backend"), "{joined}");
    assert!(joined.contains("disabled"), "{joined}");
    assert!(
        joined.contains("silently inert"),
        "the warning must say why the pair conflicts, not just that it does: {joined}"
    );
}

#[test]
fn bubble_without_isolation_warns_and_names_both_fields() {
    let config = config_with("bubble", "disabled");

    let warnings = permission_coherence_warnings(&config);

    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(warnings[0].contains("permissions.mode"), "{warnings:?}");
    assert!(warnings[0].contains("bubble"), "{warnings:?}");
    assert!(warnings[0].contains("sandbox.backend"), "{warnings:?}");
    assert!(
        warnings[0].contains("no limits"),
        "the warning must explain the conflict: {warnings:?}"
    );
}

#[test]
fn logical_backend_still_flags_the_isolation_only_fields() {
    // `logical` is not real isolation either — it is a policy gate, and the
    // three mount/lifecycle fields describe a container it never starts.
    let mut config = config_with("default", "logical");
    config.sandbox.mode = "all".to_string();

    let warnings = permission_coherence_warnings(&config);

    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(warnings[0].contains("sandbox.mode"), "{warnings:?}");
}

#[test]
fn real_isolation_backends_do_not_warn_about_their_own_fields() {
    let mut config = config_with("bubble", "docker");
    config.sandbox.mode = "all".to_string();
    config.sandbox.scope = "turn".to_string();
    config.sandbox.workspace_access = "scratch".to_string();

    assert!(
        permission_coherence_warnings(&config).is_empty(),
        "{:?}",
        permission_coherence_warnings(&config)
    );
}

#[test]
fn preset_suppression_is_preset_scoped_not_a_blanket_mute() {
    // `workspace-write` pairs a logical backend with workspace_access = "rw"
    // and is exempt as a named, curated combination. The same pair reached by
    // hand — one knob different, so no preset describes it — must still warn.
    let mut config = ArchonConfig::default();
    apply_permission_preset(&mut config, "workspace-write").expect("preset applies");
    assert!(permission_coherence_warnings(&config).is_empty());

    config.permissions.mode = "dontAsk".to_string();
    assert_eq!(derive_permission_preset(&config), CUSTOM_PRESET);

    let warnings = permission_coherence_warnings(&config);
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(
        warnings[0].contains("sandbox.workspace_access"),
        "{warnings:?}"
    );
    assert!(warnings[0].contains("logical"), "{warnings:?}");
}

#[test]
fn a_stock_config_produces_no_warnings() {
    // The load path calls this on every config in existence. Anything that
    // warns on defaults would train the warning to be ignored.
    assert!(permission_coherence_warnings(&ArchonConfig::default()).is_empty());
}

#[test]
fn unparseable_backend_reports_that_it_could_not_check() {
    let config = config_with("bubble", "podman");

    let warnings = permission_coherence_warnings(&config);

    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(
        warnings[0].contains("could not be parsed"),
        "an unparseable backend must not read as \"checked, coherent\": {warnings:?}"
    );
}
