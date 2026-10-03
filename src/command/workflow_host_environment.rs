//! Runtime-only host-child authority; never part of a catalog or run identity.
use std::{collections::BTreeMap, ffi::OsString};

use super::workflow_host_command_catalog::HostCommandResolutionContext;
use archon_workflow::EnvironmentProfileId;

// PATH resolves descendants; HOME and temporary-directory/toolchain overrides
// keep host helpers using the same locations as the invoking process.
const PROCESS_ENVIRONMENT: &[&str] = &[
    "PATH",
    "HOME",
    "TMPDIR",
    "TMP",
    "TEMP",
    "CARGO_HOME",
    "RUSTUP_HOME",
];
// On Unix, a set XDG base directory moves the configuration, data, cache and
// state locations that are otherwise derived from HOME (archon's own global
// MCP configuration and data stores among them). A child without it would use
// different locations from the process that started it.
#[cfg(unix)]
const UNIX_PROCESS_ENVIRONMENT: &[&str] = &[
    "XDG_CONFIG_HOME",
    "XDG_DATA_HOME",
    "XDG_CACHE_HOME",
    "XDG_STATE_HOME",
    "XDG_RUNTIME_DIR",
];
// Windows process, home, shell and toolchain discovery also use these names
// (the existing hook environment in archon_tools::bash_env carries them too).
#[cfg(windows)]
const WINDOWS_PROCESS_ENVIRONMENT: &[&str] = &[
    "USERPROFILE",
    "HOMEDRIVE",
    "HOMEPATH",
    "SystemRoot",
    "SystemDrive",
    "WINDIR",
    "COMSPEC",
    "PATHEXT",
    "APPDATA",
    "LOCALAPPDATA",
    "PROGRAMFILES",
    "PROGRAMFILES(X86)",
    "PROGRAMW6432",
    "PROGRAMDATA",
];

pub(crate) fn resolve(
    profile: &EnvironmentProfileId,
    context: &HostCommandResolutionContext,
) -> BTreeMap<String, OsString> {
    let mut environment = BTreeMap::new();
    let names = PROCESS_ENVIRONMENT.iter().copied();
    #[cfg(unix)]
    let names = names.chain(UNIX_PROCESS_ENVIRONMENT.iter().copied());
    #[cfg(windows)]
    let names = names.chain(WINDOWS_PROCESS_ENVIRONMENT.iter().copied());
    let copy = |environment: &mut BTreeMap<_, _>, name: &str| {
        if let Some(value) = std::env::var_os(name) {
            environment.insert(name.to_string(), value);
        }
    };
    for name in names {
        copy(&mut environment, name);
    }
    // None: the skeleton freeze, requirements trace (git only; its argv never
    // passes --falsify, so no verifier runs) and frozen-chain verification run
    // no project code, so they get the process essentials and nothing else.
    // FreezeProvider: acceptance probes run project checks and the body/set
    // fidelity judges call the provider, so they also get provider settings
    // and the configured allowlist. Scratch preparation fails on a missing
    // value, by name, before any check runs; output is redacted in
    // workflow_host_secrets.
    match profile {
        EnvironmentProfileId::None => {}
        EnvironmentProfileId::FreezeProvider => {
            for name in &context.acceptance_environment_allowlist {
                copy(&mut environment, name);
            }
            environment.extend(
                context
                    .freeze_provider_environment
                    .iter()
                    .map(|(name, value)| (name.clone(), OsString::from(value))),
            );
        }
    }
    environment
}
