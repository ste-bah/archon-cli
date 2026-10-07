//! A pre-validation recorded binding cannot put refused names into the
//! environment used to identify the site's tools.

use super::*;

#[test]
fn recorded_execution_control_names_are_not_forwarded_to_site_commands() {
    const REFUSED: &[(&str, &str)] = &[
        ("LD_PRELOAD", "recorded-loader-value"),
        ("DYLD_INSERT_LIBRARIES", "recorded-dyld-value"),
        ("CARGO_ENCODED_RUSTFLAGS", "recorded-rustflags-value"),
    ];
    let mut host = BTreeMap::from([("PATH".into(), "/usr/bin".into())]);
    host.extend(
        REFUSED
            .iter()
            .map(|(name, value)| ((*name).to_string(), (*value).to_string())),
    );
    let binding = NativeBinding {
        policy: archon_workflow::acceptance_scratch::ScratchPolicy {
            repository: Path::new("/repository").into(),
            project: Path::new("/project").into(),
            task_root: Path::new("/project/tasks").into(),
            scratch_parent: Path::new("/scratch").into(),
            project_inputs: Vec::new(),
            project_input_excludes: Vec::new(),
            combined: false,
            toolchain_path: "/usr/bin".into(),
            environment: BTreeMap::new(),
            environment_allowlist: REFUSED
                .iter()
                .map(|(name, _)| (*name).to_string())
                .collect(),
            cargo_seed: None,
            timeout_secs: 10,
            output_bytes: 1024,
            scratch_bytes: 1024,
            build_cache: None,
        },
        source_commit: "recorded-source".into(),
        external_data_roots: Vec::new(),
    };
    // A resume consumes the persisted shape; it does not recapture config.
    let recorded: NativeBinding =
        serde_json::from_slice(&serde_json::to_vec(&binding).unwrap()).unwrap();
    let probe = HostProbe::new(
        Path::new("/project").into(),
        Path::new("/repository").into(),
        Site::Scratch(Box::new(recorded)),
    )
    .with_host_environment(host);

    let (environment, _) = site_environment(&probe);

    for (name, value) in REFUSED {
        assert!(!environment.contains_key(*name), "{name} reached a command");
        assert!(
            !environment.values().any(|candidate| candidate == *value),
            "{name}'s recorded value reached a command"
        );
    }
}
