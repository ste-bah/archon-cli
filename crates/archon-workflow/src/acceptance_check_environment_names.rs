//! Names provided by the shell or acceptance sites, independent of host state.
use super::{CheckPolicy, DEFAULT_BOUND, PROXY_VARIABLES, SYSTEM_VARIABLES};
pub fn shell_bound_name(name: &str, policy_names: &[&str]) -> bool {
    const SITE_AND_SHELL: &[&str] = &[
        "HOME",
        "PATH",
        "PWD",
        "OLDPWD",
        "TMPDIR",
        "TMP",
        "TEMP",
        "USER",
        "LOGNAME",
        "SHELL",
        "SHLVL",
        "IFS",
        "OPTIND",
        "OPTARG",
        "PPID",
        "PID",
        "UID",
        "EUID",
        "BASH",
        "HOSTNAME",
        "HOSTTYPE",
        "MACHTYPE",
        "OSTYPE",
        "PS1",
        "PS2",
        "PS4",
        "ZSH_VERSION",
        "LINENO",
        "RANDOM",
        "SECONDS",
        "_",
        "FUNCNAME",
        "GROUPS",
        "SHELLOPTS",
        "BASHPID",
        "EPOCHSECONDS",
        "EPOCHREALTIME",
    ];
    name.starts_with("BASH_")
        || SITE_AND_SHELL.contains(&name)
        || DEFAULT_BOUND.contains(&name)
        || PROXY_VARIABLES.contains(&name)
        || SYSTEM_VARIABLES.contains(&name)
        || archon_tools::workflow_resource_env::WORKFLOW_RESOURCE_ENV_KEYS.contains(&name)
        || archon_tools::build_cache_env::toolchain_cache_env_keys().any(|key| key == name)
        || policy_names.contains(&name)
}

pub fn policy_bound_names(policy: &CheckPolicy) -> Vec<&str> {
    policy
        .bound
        .keys()
        .map(String::as_str)
        .chain(policy.forwarded.iter().map(String::as_str))
        .collect()
}
