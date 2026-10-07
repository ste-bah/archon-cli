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
//! 3. the site's own directories (its `HOME` -- the host's at the direct
//!    site, which has no filesystem sandbox to make a fresh one a boundary,
//!    a fresh one at a probe's copy and a scratch --, a scratch's `TMPDIR`,
//!    `CARGO_HOME` and build directory), which nothing above displaces.
//!
//! A configured `[workflow.acceptance_execution]` section is the policy of
//! its sites ([`CheckPolicy::configured`]). A site with no section has the
//! default policy ([`CheckPolicy::default_for`]): the host's `PATH`, and from
//! the host only [`DEFAULT_BOUND`] (locale, time zone, terminal type, user
//! name, temporary directory, the Rust toolchain's homes and build directory,
//! and the non-secret locators of other toolchains and certificate bundles)
//! and [`PROXY_VARIABLES`] whose value is a bare address, never a
//! credential-shaped name ([`credential_shaped`]); the toolchain homes a
//! tool finds under the host's home are named outright. It forwards nothing
//! else: an operator variable a check needs is named in
//! `environment_allowlist`, and never reaches a check by default.
//!
//! Output never changes a verifier's real verdict. On failure, a separate
//! [`withheld_note`] lists withheld names occurring as case-sensitive substrings
//! anywhere in stdout/stderr, so the agent can ask the operator to allowlist
//! needed data. This is a diagnostic, never evidence of the failure's cause.

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::path::Path;

use crate::acceptance_scratch::ScratchPolicy;

#[cfg(test)]
#[path = "acceptance_check_environment_note_tests.rs"]
mod note_channel_tests;

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
        "CommonProgramFiles(x86)",
        "CommonProgramW6432",
        "NUMBER_OF_PROCESSORS",
        "PROCESSOR_ARCHITECTURE",
        "OS",
        "USERPROFILE",
        "APPDATA",
        "LOCALAPPDATA",
        "HOMEDRIVE",
        "HOMEPATH",
        "USERNAME",
        "COMPUTERNAME",
        "PSModulePath",
        "ALLUSERSPROFILE",
    ]
} else {
    &[]
};

/// Host variables the default policy binds, when the host has them. None is
/// a secret: each names a locale, a user, a directory, a toolchain version or
/// a certificate bundle's path, never a credential.
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
    // Python: the active virtual or conda environment's directory, the module
    // search path, pyenv's root directory and its selected version.
    "VIRTUAL_ENV",
    "CONDA_PREFIX",
    "PYTHONPATH",
    "PYENV_ROOT",
    "PYENV_VERSION",
    // Node: nvm's directory and the module search path.
    "NVM_DIR",
    "NODE_PATH",
    // Go: the workspace, the toolchain root and the module cache directory.
    "GOPATH",
    "GOROOT",
    "GOMODCACHE",
    // JVM: the JDK's directory and Gradle's cache directory.
    "JAVA_HOME",
    "GRADLE_USER_HOME",
    // Certificate bundles: paths to public CA certificates, never keys.
    "SSL_CERT_FILE",
    "SSL_CERT_DIR",
    "REQUESTS_CA_BUNDLE",
    "NODE_EXTRA_CA_CERTS",
];

/// Proxy addresses the default policy binds, when the host has them and the
/// value is a bare address ([`bare_proxy`]): an address without credentials
/// is no secret, and a check that downloads behind a proxy needs it.
pub const PROXY_VARIABLES: &[&str] = &[
    "HTTP_PROXY",
    "HTTPS_PROXY",
    "NO_PROXY",
    "http_proxy",
    "https_proxy",
    "no_proxy",
];

/// Whether `name` is shaped like a credential: a name Issue 282's data rule
/// accepts (a documented data suffix) or one ending in a secret's suffix.
/// The default policy never binds one.
pub fn credential_shaped(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    archon_shell::data_environment::check_data_variable(name).is_ok()
        || ["_TOKEN", "_KEY", "_SECRET", "_PASSWORD", "_PAT"]
            .iter()
            .any(|suffix| upper.ends_with(suffix))
}

/// Whether proxy variable `name`'s `value` is a bare address: no `@` (a
/// `user:pass@`), `?` or `#` anywhere (a query or fragment can carry a
/// token), and for a proxy URL no path but `/`. A `NO_PROXY` list is hosts
/// and CIDR blocks (`10.0.0.0/8`), so it has no path rule.
fn bare_proxy(name: &str, value: &str) -> bool {
    if value.contains(['@', '?', '#']) {
        return false;
    }
    if name.eq_ignore_ascii_case("NO_PROXY") {
        return true;
    }
    let rest = value.split_once("://").map_or(value, |(_, rest)| rest);
    rest.split_once('/').is_none_or(|(_, path)| path.is_empty())
}

/// Windows variables that name the machine or its user: kept for a check,
/// never part of a build identity, which must not be one machine's.
pub const MACHINE_VARIABLES: &[&str] = &["USERNAME", "COMPUTERNAME"];

/// The Windows profile variables of a site with its own `home` (a scratch, a
/// probe's copy): USERPROFILE, APPDATA and LOCALAPPDATA are `home`, and
/// HOMEDRIVE and HOMEPATH split it, so no tool falls back to the operator's
/// profile. None outside Windows.
pub fn profile_bindings(home: &Path) -> Vec<(&'static str, std::path::PathBuf)> {
    if !cfg!(windows) {
        return Vec::new();
    }
    let mut bindings: Vec<(&'static str, std::path::PathBuf)> =
        ["USERPROFILE", "APPDATA", "LOCALAPPDATA"]
            .into_iter()
            .map(|name| (name, home.to_path_buf()))
            .collect();
    if let Some(std::path::Component::Prefix(prefix)) = home.components().next() {
        let drive = std::path::PathBuf::from(prefix.as_os_str());
        if let Ok(rest) = home.strip_prefix(&drive) {
            bindings.push(("HOMEPATH", rest.to_path_buf()));
        }
        bindings.push(("HOMEDRIVE", drive));
    }
    bindings
}

/// What a site's policy gives a check.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
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

    /// Approved dispatch locators supersede config/host locators for this lease.
    /// Unknown variables remain withheld; a dispatch overlay cannot forward data.
    pub fn bind_dispatch(&mut self, overrides: &[(String, String)]) {
        for (name, value) in overrides {
            let approved = DEFAULT_BOUND
                .iter()
                .copied()
                .chain(archon_tools::build_cache_env::toolchain_cache_env_keys())
                .any(|key| {
                    if cfg!(windows) {
                        key.eq_ignore_ascii_case(name)
                    } else {
                        key == name
                    }
                });
            if name == "PATH" || (cfg!(windows) && name.eq_ignore_ascii_case("PATH")) {
                self.toolchain_path = Some(value.clone());
            } else if approved && !credential_shaped(name) {
                while let Some((previous, _)) = lookup(&self.bound, name) {
                    let previous = previous.clone();
                    self.bound.remove(&previous);
                }
                self.bound.insert(name.clone(), value.clone());
            }
        }
    }

    /// The policy of a site with no section, read from `host`.
    pub fn default_for(host: &BTreeMap<String, String>) -> Self {
        let proxies = (PROXY_VARIABLES.iter())
            .filter_map(|name| lookup(host, name))
            .filter(|(name, value)| bare_proxy(name, value));
        let mut bound: BTreeMap<String, String> = (DEFAULT_BOUND.iter())
            .filter_map(|name| lookup(host, name))
            .chain(proxies)
            .filter(|(name, _)| !credential_shaped(name))
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
pub(crate) fn lookup<'a>(
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
    unicode_environment(std::env::vars_os())
}

fn unicode_environment(
    variables: impl IntoIterator<Item = (std::ffi::OsString, std::ffi::OsString)>,
) -> BTreeMap<String, String> {
    variables
        .into_iter()
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

/// The policy-selected check environment for an agent-authored host verifier.
/// Uses the same builder as direct/scratch acceptance, with the host HOME.
/// Environment construction must succeed before a command can be built.
pub struct CommandEnvironment {
    variables: BTreeMap<String, String>,
    withheld: BTreeSet<String>,
    remedy: &'static str,
}

impl CommandEnvironment {
    pub fn capture(policy: Option<&CheckPolicy>) -> Result<Self, String> {
        Self::capture_with_dispatch(policy, &[])
    }

    /// Capture the OS environment once and apply approved dispatch locators.
    pub fn capture_with_dispatch(
        policy: Option<&CheckPolicy>,
        dispatch: &[(String, String)],
    ) -> Result<Self, String> {
        // One OS snapshot for bindings and withheld names. A non-Unicode
        // value cannot be forwarded, but its Unicode name still needs a note.
        let host: Vec<_> = std::env::vars_os().collect();
        let names: BTreeSet<String> = host
            .iter()
            .filter_map(|(name, _)| name.to_str().map(str::to_owned))
            .collect();
        let unicode = unicode_environment(host);
        let mut effective = policy
            .cloned()
            .unwrap_or_else(|| CheckPolicy::default_for(&unicode));
        effective.bind_dispatch(dispatch);
        let mut environment = Self::from_host(&unicode, Some(&effective))?;
        environment.withheld.extend(
            names
                .into_iter()
                .filter(|name| lookup(&environment.variables, name).is_none()),
        );
        Ok(environment)
    }

    /// The policy is operator-owned, never taken from verifier/task text.
    /// `None` means the operator configured no policy.
    pub fn from_host(
        host: &BTreeMap<String, String>,
        policy: Option<&CheckPolicy>,
    ) -> Result<Self, String> {
        let site: Vec<(&str, &Path)> = lookup(host, "HOME")
            .map(|(_, home)| ("HOME", Path::new(home)))
            .into_iter()
            .collect();
        // Host verifiers share the leased machine toolchains. A data allowlist
        // must not select a cold cache. Start with the builder's approved host
        // bindings, then apply explicit operator/dispatch bindings. Scratch
        // environments still use their own site bindings and directory homes.
        let mut effective = CheckPolicy::default_for(host);
        if let Some(policy) = policy {
            effective.toolchain_path = policy.toolchain_path.clone();
            for (name, value) in &policy.bound {
                while let Some((previous, _)) = lookup(&effective.bound, name) {
                    let previous = previous.clone();
                    effective.bound.remove(&previous);
                }
                effective.bound.insert(name.clone(), value.clone());
            }
            effective.forwarded = policy.forwarded.clone();
        }
        let variables = check_environment(host, &effective, &site)
            .map_err(|reason| format!("check command environment could not be built: {reason}"))?;
        Ok(Self {
            withheld: withheld(host, &variables),
            remedy: "Supply the needed names in the operator-owned CheckPolicy.forwarded passed to this verifier",
            variables,
        })
    }

    /// Describe the actual operator policy source this runner consumes.
    pub fn with_remedy(mut self, remedy: &'static str) -> Self {
        self.remedy = remedy;
        self
    }

    /// Every child still passes through the process-wide spawn boundary.
    pub fn command(&self, program: impl AsRef<OsStr>) -> std::process::Command {
        let mut command = archon_shell::spawn::command(program);
        command.env_clear().envs(&self.variables);
        command
    }

    pub fn tokio_command(&self, program: impl AsRef<OsStr>) -> tokio::process::Command {
        tokio::process::Command::from(self.command(program))
    }

    /// Call only for a failed verifier. Persist this separate diagnostic with
    /// the result; stdout/stderr must never change the verdict (Issue 349).
    pub fn note(&self, outputs: &[&[u8]]) -> Option<String> {
        let note = withheld::withheld_note_with_remedy(outputs, &self.withheld, self.remedy);
        if let Some(note) = &note {
            tracing::warn!(target: "archon_workflow::check_environment", "{note}");
        }
        note
    }
}

#[path = "acceptance_check_environment_policy.rs"]
mod policy;
pub use policy::{
    NO_RUN_POLICY_REMEDY, RUN_POLICY_REMEDY, policy_for_run, validate_operator_bindings,
};

#[path = "acceptance_check_environment_withheld.rs"]
mod withheld;
pub use withheld::{withheld, withheld_note};

#[cfg(test)]
#[path = "acceptance_check_environment_tests.rs"]
mod tests;
