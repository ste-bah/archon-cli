/// Abbreviate a CamelCase name by taking the first letter of each component,
/// keeping short words as-is.
pub(super) fn abbreviate(name: &str) -> String {
    // Split CamelCase into components.
    let parts = split_camel_case(name);
    if parts.len() <= 1 {
        // Not CamelCase — try component abbreviation, then known abbreviation.
        return abbreviate_component(name);
    }

    let mut abbrev = String::new();
    for part in &parts {
        abbrev.push_str(&abbreviate_component(part));
    }
    abbrev
}

/// Abbreviate a single CamelCase component word.
fn abbreviate_component(word: &str) -> String {
    // Known component abbreviations.
    match word.to_lowercase().as_str() {
        "service" => "Svc".to_string(),
        "handler" => "Hnd".to_string(),
        "manager" => "Mgr".to_string(),
        "controller" => "Ctl".to_string(),
        "store" => "Str".to_string(),
        "repository" => "Repo".to_string(),
        "validator" => "Val".to_string(),
        "middleware" => "MW".to_string(),
        "gateway" => "GW".to_string(),
        "factory" => "Fct".to_string(),
        "builder" => "Bld".to_string(),
        "provider" => "Prv".to_string(),
        "listener" => "Lsn".to_string(),
        "collector" => "Col".to_string(),
        "interceptor" => "Icp".to_string(),
        "transformer" => "Xfm".to_string(),
        "notification" => "Ntf".to_string(),
        "connection" => "Conn".to_string(),
        "migration" => "Mig".to_string(),
        "configuration" => "Cfg".to_string(),
        "authentication" => "Auth".to_string(),
        "postgres" | "postgresql" => "Pg".to_string(),
        "session" => "Sess".to_string(),
        "response" => "Rsp".to_string(),
        "request" => "Req".to_string(),
        "context" => "Ctx".to_string(),
        "feature" => "Feat".to_string(),
        "policy" => "Pol".to_string(),
        "limiter" => "Lim".to_string(),
        "runner" => "Run".to_string(),
        "checker" | "check" => "Chk".to_string(),
        "metrics" => "Mtr".to_string(),
        "health" => "Hlth".to_string(),
        "logging" => "Log".to_string(),
        "error" => "Err".to_string(),
        "retry" => "Rty".to_string(),
        "token" => "Tok".to_string(),
        "cache" => "Cch".to_string(),
        "event" => "Evt".to_string(),
        _ => {
            // Unknown component: just first letter (aggressive abbreviation).
            word.chars()
                .next()
                .map(|c| c.to_ascii_uppercase().to_string())
                .unwrap_or_default()
        }
    }
}

pub(super) fn split_camel_case(s: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    for ch in s.chars() {
        if ch.is_uppercase() && !current.is_empty() {
            parts.push(current);
            current = String::new();
        }
        current.push(ch);
    }
    if !current.is_empty() {
        parts.push(current);
    }
    parts
}

pub(super) fn apply_known_abbrev(word: &str) -> String {
    match word.to_lowercase().as_str() {
        "authentication" | "authenticator" => "auth".to_string(),
        "database" => "db".to_string(),
        "repository" => "repo".to_string(),
        "configuration" | "config" => "cfg".to_string(),
        "implementation" => "impl".to_string(),
        "function" => "fn".to_string(),
        "structure" => "struct".to_string(),
        "enumeration" => "enum".to_string(),
        "module" => "mod".to_string(),
        _ => {
            // Short words: keep as-is.
            if word.len() < 5 {
                word.to_string()
            } else {
                // First 4 chars.
                word[..word.len().min(4)].to_string()
            }
        }
    }
}
