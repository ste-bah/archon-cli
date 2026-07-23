const SENSITIVE_PATTERNS: &[&str] = &[
    "ANTHROPIC_API_KEY",
    "ANTHROPIC_AUTH_TOKEN",
    "_TOKEN",
    "_SECRET",
    "_KEY",
    "_PASSWORD",
    "_CREDENTIAL",
];

const PASSTHROUGH_VARS: &[&str] = &[
    "PATH",
    "HOME",
    "USER",
    "SHELL",
    "LANG",
    "LC_ALL",
    "TERM",
    "DISPLAY",
    "XDG_RUNTIME_DIR",
    "DBUS_SESSION_BUS_ADDRESS",
    "SSH_AUTH_SOCK",
    "EDITOR",
    "VISUAL",
    "TMPDIR",
    "TMP",
    "TEMP",
];

pub fn sanitized_env() -> Vec<(String, String)> {
    let mut env = Vec::new();

    for (key, value) in std::env::vars() {
        if PASSTHROUGH_VARS.contains(&key.as_str()) {
            env.push((key, value));
            continue;
        }

        let upper = key.to_uppercase();
        let is_sensitive = SENSITIVE_PATTERNS
            .iter()
            .any(|pattern| upper.contains(pattern));

        if !is_sensitive {
            env.push((key, value));
        }
    }

    env
}

pub(super) fn ensure_env_default(env: &mut Vec<(String, String)>, key: &str, value: &str) {
    if !env.iter().any(|(existing, _)| existing == key) {
        env.push((key.to_string(), value.to_string()));
    }
}
