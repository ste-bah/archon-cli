/// Evaluate a hook condition expression against a JSON input payload.
///
/// # Syntax
///
/// - `""` or `"*"` — always matches (unconditional)
/// - `"ToolName"` — matches when `input["tool_name"] == ToolName`
/// - `"ToolName(pattern)"` — matches when tool_name matches AND the first
///   command argument matches the glob `pattern`
///
/// # Glob patterns
///
/// `*` matches any sequence of characters. All other characters are literal.
///
/// # Examples
///
/// ```text
/// evaluate("Bash(git *)", {"tool_name":"Bash","tool_input":{"command":"git commit"}}) → true
/// evaluate("Bash(git *)", {"tool_name":"Bash","tool_input":{"command":"rm -rf /"}})  → false
/// evaluate("Read",        {"tool_name":"Read","tool_input":{"path":"/foo"}})          → true
/// evaluate("",            anything)                                                    → true
/// ```
pub fn evaluate(condition: &str, input: &serde_json::Value) -> bool {
    let condition = condition.trim();

    // Empty condition or bare wildcard — always match.
    if condition.is_empty() || condition == "*" {
        return true;
    }

    if let Some(paren_pos) = condition.find('(') {
        // Format: ToolName(pattern)
        let tool_name_pat = &condition[..paren_pos];
        let rest = &condition[paren_pos + 1..];
        let pattern = rest.trim_end_matches(')');

        let actual_tool = input
            .get("tool_name")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if actual_tool != tool_name_pat {
            return false;
        }

        // Extract command string from tool_input.command (or tool_input.cmd)
        let command_str = input
            .get("tool_input")
            .and_then(|ti| ti.get("command").or_else(|| ti.get("cmd")))
            .and_then(|v| v.as_str())
            .unwrap_or("");

        glob_match(pattern, command_str)
    } else {
        // Format: just ToolName — exact match against tool_name
        let actual_tool = input
            .get("tool_name")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        actual_tool == condition
    }
}

/// Simple glob match: `*` matches any sequence of characters.
/// All other characters are literals.
pub(crate) fn glob_match(pattern: &str, text: &str) -> bool {
    if pattern == "*" {
        return true;
    }
    if !pattern.contains('*') {
        return text == pattern;
    }

    // Split on `*` and match each segment in order.
    let parts: Vec<&str> = pattern.split('*').collect();
    let mut remaining = text;

    for (i, part) in parts.iter().enumerate() {
        if part.is_empty() {
            // Leading, trailing, or consecutive `*` — skip.
            continue;
        }
        if i == 0 {
            // First segment: text must start with this literal.
            if !remaining.starts_with(part) {
                return false;
            }
            remaining = &remaining[part.len()..];
        } else if i == parts.len() - 1 {
            // Last segment: remaining text must end with this literal.
            return remaining.ends_with(part);
        } else {
            // Middle segment: find the literal anywhere in remaining.
            match remaining.find(part) {
                Some(pos) => remaining = &remaining[pos + part.len()..],
                None => return false,
            }
        }
    }

    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_matches_all() {
        let input = serde_json::json!({"tool_name": "Bash"});
        assert!(evaluate("", &input));
        assert!(evaluate("*", &input));
    }

    #[test]
    fn tool_name_exact() {
        let bash = serde_json::json!({"tool_name": "Bash"});
        let read = serde_json::json!({"tool_name": "Read"});
        assert!(evaluate("Bash", &bash));
        assert!(!evaluate("Bash", &read));
    }

    #[test]
    fn glob_match_wildcard() {
        assert!(glob_match("*", "anything"));
        assert!(glob_match("git *", "git commit -m fix"));
        assert!(!glob_match("git *", "rm -rf /"));
        assert!(glob_match("any*", "anything here"));
        assert!(!glob_match("nothing*", "anything here"));
    }
}
