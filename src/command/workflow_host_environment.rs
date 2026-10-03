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
    #[cfg(windows)]
    let names = names.chain(WINDOWS_PROCESS_ENVIRONMENT.iter().copied());
    for name in names.chain(
        context
            .acceptance_environment_allowlist
            .iter()
            .map(String::as_str),
    ) {
        if let Some(value) = std::env::var_os(name) {
            environment.insert(name.to_string(), value);
        }
    }
    // None: skeleton/frozen-chain checks read files, but requirements trace
    // executes falsification verifiers, so all retain process essentials.
    // FreezeProvider: acceptance probes and body/set fidelity judges also need
    // provider settings. Both carry the configured allowlist; the acceptance
    // runner diagnoses missing values per check at the env_clear boundary.
    match profile {
        EnvironmentProfileId::None => {}
        EnvironmentProfileId::FreezeProvider => environment.extend(
            context
                .freeze_provider_environment
                .iter()
                .map(|(name, value)| (name.clone(), OsString::from(value))),
        ),
    }
    environment
}
