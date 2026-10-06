//! The one environment rule for an acceptance check, at every site (Issue 345).
//!
//! A check is a command an agent wrote, and the host runs it. Whatever site
//! runs it -- the configured scratch observation, the live checkout of a run
//! with no `[workflow.acceptance_execution]` section (the direct site), or a
//! freeze probe's hermetic copy -- it gets an allowlisted environment and
//! never the host's own:
//!
//! 1. the platform's own process variables, where the host has them
//!    ([`SYSTEM_VARIABLES`]: Windows only, where a process without
//!    `SystemRoot` cannot load its sockets or crypto providers);
//! 2. its policy's bindings ([`CheckPolicy`]): `PATH`, the values the policy
//!    binds outright, and the host variables it forwards on purpose. Each
//!    forwarded name must pass Issue 282's data rule
//!    (`archon_shell::data_environment::check_data_variable`) and be set on
//!    the host, else the check does not run and the error names it
//!    ([`forwarded_values`]);
//! 3. the site's own directories (a fresh `HOME`, a scratch's `TMPDIR`,
//!    `CARGO_HOME` and build directory), which nothing above displaces.
//!
//! A configured `[workflow.acceptance_execution]` section is the policy of
//! its sites ([`CheckPolicy::configured`]). A site with no section has the
//! default policy ([`CheckPolicy::default_for`]): the host's `PATH`, and from
//! the host only [`DEFAULT_BOUND`] (locale, time zone, terminal type, user
//! name, temporary directory and the Rust toolchain's homes and build
//! directory), with the toolchain homes a tool finds under the host's home
//! named outright, because the check is given a fresh one. It forwards
//! nothing else: an operator variable a check needs is named in
//! `environment_allowlist`, and never reaches a check by default.
//!
//! Before Issue 345 a site with no section gave a check every host variable
//! but Archon's own keys. A check that read one of them now fails; when its
//! text or output names a host variable the site withheld ([`withheld`]),
//! that failure is no verdict but an operational error naming the variable
//! ([`withheld_error`]).

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use crate::acceptance_scratch::ScratchPolicy;

/// Host variables every site keeps, when the host has them: what a process
/// needs to start on this platform. None outside Windows.
pub const SYSTEM_VARIABLES: &[&str] = if cfg!(windows) {
    &[
        "SystemRoot",
        "windir",
        "SystemDrive",
        "PATHEXT",
        "ComSpec",
        "TEMP",
        "TMP",
        "ProgramFiles",
        "ProgramFiles(x86)",
        "ProgramW6432",
        "ProgramData",
        "CommonProgramFiles",
        "NUMBER_OF_PROCESSORS",
        "PROCESSOR_ARCHITECTURE",
        "OS",
    ]
} else {
    &[]
};

/// Host variables the default policy binds, when the host has them.
pub const DEFAULT_BOUND: &[&str] = &[
    "LANG",
    "LANGUAGE",
    "LC_ALL",
    "LC_ADDRESS",
    "LC_COLLATE",
    "LC_CTYPE",
    "LC_IDENTIFICATION",
    "LC_MEASUREMENT",
    "LC_MESSAGES",
    "LC_MONETARY",
    "LC_NAME",
    "LC_NUMERIC",
    "LC_PAPER",
    "LC_TELEPHONE",
    "LC_TIME",
    "TZ",
    "TERM",
    "USER",
    "LOGNAME",
    "TMPDIR",
    "CARGO_HOME",
    "RUSTUP_HOME",
    "RUSTUP_TOOLCHAIN",
    "CARGO_TARGET_DIR",
];

/// Variables a shell sets for itself: never withheld, whatever the host has.
const SHELL_OWN: &[&str] = &["PWD", "OLDPWD", "SHLVL", "_"];

/// What a site's policy gives a check.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CheckPolicy {
    /// `PATH`; `None` only when the host itself has no PATH (default policy).
    pub toolchain_path: Option<String>,
    /// Values bound outright.
    pub bound: BTreeMap<String, String>,
    /// Host variables forwarded on purpose; each one is required.
    pub forwarded: Vec<String>,
}

impl CheckPolicy {
    /// The policy of a configured `[workflow.acceptance_execution]` section.
    pub fn configured(policy: &ScratchPolicy) -> Self {
        Self {
            toolchain_path: Some(policy.toolchain_path.clone()),
            bound: policy.environment.clone(),
            forwarded: policy.environment_allowlist.clone(),
        }
    }

    /// The policy of a site with no section, read from `host`.
    pub fn default_for(host: &BTreeMap<String, String>) -> Self {
        let mut bound: BTreeMap<String, String> = DEFAULT_BOUND
            .iter()
            .filter_map(|name| lookup(host, name))
            .map(|(name, value)| (name.clone(), value.clone()))
            .collect();
        let home_name = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
        if let Some((_, home)) = lookup(host, home_name) {
            for (name, under) in [("CARGO_HOME", ".cargo"), ("RUSTUP_HOME", ".rustup")] {
                let dir = Path::new(home).join(under);
                if lookup(&bound, name).is_none() && dir.is_dir() {
                    bound.insert(name.into(), dir.to_string_lossy().into_owned());
                }
            }
        }
        Self {
            toolchain_path: lookup(host, "PATH").map(|(_, path)| path.clone()),
            bound,
            forwarded: Vec::new(),
        }
    }
}

/// `name` in `environment`: exactly, and ignoring ASCII case on Windows,
/// where `Path` is PATH.
fn lookup<'a>(
    environment: &'a BTreeMap<String, String>,
    name: &str,
) -> Option<(&'a String, &'a String)> {
    environment.get_key_value(name).or_else(|| {
        cfg!(windows)
            .then(|| (environment.iter()).find(|(key, _)| key.eq_ignore_ascii_case(name)))
            .flatten()
    })
}

/// The host's environment, as a map: every variable whose name and value are
/// Unicode (a check's environment is built from it, never handed it whole).
pub fn host_environment() -> BTreeMap<String, String> {
    std::env::vars_os()
        .filter_map(|(name, value)| Some((name.into_string().ok()?, value.into_string().ok()?)))
        .collect()
}

/// [`SYSTEM_VARIABLES`] as `host` has them.
pub fn system_variables(host: &BTreeMap<String, String>) -> BTreeMap<String, String> {
    SYSTEM_VARIABLES
        .iter()
        .filter_map(|name| lookup(host, name))
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect()
}

/// A check's variables at a site, all but the forwarded values: the system
/// variables, then the policy's PATH and bound values, then `site`'s own
/// directories. A site's record of what it gives (a build identity, a
/// listing) is this, never the forwarded secrets.
pub fn site_variables(
    host: &BTreeMap<String, String>,
    policy: &CheckPolicy,
    site: &[(&str, &Path)],
) -> BTreeMap<String, String> {
    let mut environment = system_variables(host);
    environment.extend(policy.bound.clone());
    if let Some(path) = &policy.toolchain_path {
        environment.insert("PATH".into(), path.clone());
    }
    for (name, path) in site {
        environment.insert((*name).into(), path.to_string_lossy().into_owned());
    }
    environment
}

/// The values of the variables a policy forwards, from `host`. A name that
/// fails Issue 282's data rule, or that the host has no Unicode value for, is
/// an error naming it: the check does not run.
pub fn forwarded_values(
    host: &BTreeMap<String, String>,
    names: &[String],
) -> Result<BTreeMap<String, String>, String> {
    names
        .iter()
        .map(|name| {
            archon_shell::data_environment::check_data_variable(name)
                .map_err(|reason| format!("acceptance environment allowlist: {reason}"))?;
            lookup(host, name)
                .map(|(_, value)| (name.clone(), value.clone()))
                .ok_or_else(|| format!(
                    "allowlisted environment variable '{name}' is absent or not Unicode; set {name} in the environment archon is started with and retry the check"
                ))
        })
        .collect()
}

/// A check's whole environment at a site: [`site_variables`] and the
/// forwarded values.
pub fn check_environment(
    host: &BTreeMap<String, String>,
    policy: &CheckPolicy,
    site: &[(&str, &Path)],
) -> Result<BTreeMap<String, String>, String> {
    if policy.toolchain_path.is_none() {
        return Err(
            "the host environment has no PATH, so a check could find no program; start archon with PATH set and retry the check"
                .into(),
        );
    }
    let mut environment = site_variables(host, policy, site);
    environment.extend(forwarded_values(host, &policy.forwarded)?);
    Ok(environment)
}

/// The host variables a check given `environment` does not get.
pub fn withheld(
    host: &BTreeMap<String, String>,
    environment: &BTreeMap<String, String>,
) -> BTreeSet<String> {
    (host.keys())
        .filter(|name| !SHELL_OWN.contains(&name.as_str()) && lookup(environment, name).is_none())
        .cloned()
        .collect()
}

/// The operational error of a check that failed and whose text or output
/// names a host variable the site withheld, naming it; `None` when it names
/// none. Names are matched as whole words, so `MY_PAT` is not `MY_PATH`.
pub fn withheld_error(texts: &[&[u8]], withheld: &BTreeSet<String>) -> Option<String> {
    let texts: Vec<String> = (texts.iter())
        .map(|text| String::from_utf8_lossy(text).into_owned())
        .collect();
    let named: BTreeSet<&str> = (texts.iter())
        .flat_map(|text| text.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_')))
        .filter_map(|word| withheld.get(word).map(String::as_str))
        .collect();
    if named.is_empty() {
        return None;
    }
    let list = named.into_iter().collect::<Vec<_>>().join(", ");
    Some(format!(
        "the check failed and names the host variable(s) {list}, which this acceptance site does not give a check (Issue 345: a check gets PATH, the locale, the toolchain homes and only the host variables its policy forwards), so the failure is no verdict. If the check needs them, name them in [workflow.acceptance_execution] environment_allowlist; otherwise the check must not read them"
    ))
}

#[cfg(test)]
#[path = "acceptance_check_environment_tests.rs"]
mod tests;
