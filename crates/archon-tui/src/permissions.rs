/// Permission prompt for inline display.
#[derive(Debug, Clone)]
pub struct PermissionPrompt {
    pub tool_name: String,
    pub description: String,
}

impl PermissionPrompt {
    pub fn format(&self) -> String {
        format!(
            "Allow {} to {}? [y/n/always] ",
            self.tool_name, self.description
        )
    }
}

/// Parse user response to permission prompt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PermissionResponse {
    Allow,
    Deny,
    AlwaysAllow,
}

pub fn parse_permission_response(input: &str) -> Option<PermissionResponse> {
    match input.trim().to_lowercase().as_str() {
        "y" | "yes" => Some(PermissionResponse::Allow),
        "n" | "no" => Some(PermissionResponse::Deny),
        "a" | "always" => Some(PermissionResponse::AlwaysAllow),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_responses() {
        assert_eq!(
            parse_permission_response("y"),
            Some(PermissionResponse::Allow)
        );
        assert_eq!(
            parse_permission_response("n"),
            Some(PermissionResponse::Deny)
        );
        assert_eq!(
            parse_permission_response("always"),
            Some(PermissionResponse::AlwaysAllow)
        );
        assert_eq!(parse_permission_response("maybe"), None);
    }
}
