use archon_workflow::WorkflowV2Result;

/// Schemes whose artifacts genuinely live outside the filesystem. An
/// allowlist, not a shape check: existence validation is one leg of
/// completion credit, so an arbitrary `whatever:` prefix must not bypass it.
const NONFILESYSTEM_SCHEMES: &[&str] = &["inline", "data", "http", "https", "mcp"];

pub(super) fn is_nonfilesystem_artifact_ref(raw: &str) -> bool {
    let Some((scheme, _)) = raw.trim().split_once(':') else {
        return false;
    };
    NONFILESYSTEM_SCHEMES
        .iter()
        .any(|known| scheme.eq_ignore_ascii_case(known))
}

pub(super) fn retain_filesystem_artifacts(result: &mut WorkflowV2Result) {
    result
        .artifacts
        .retain(|artifact| !is_nonfilesystem_artifact_ref(&artifact.path));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_only_allowlisted_nonfilesystem_scheme_refs() {
        assert!(is_nonfilesystem_artifact_ref("inline:data.items"));
        assert!(is_nonfilesystem_artifact_ref("https://example.test/report"));
        assert!(is_nonfilesystem_artifact_ref("mcp:chart-snapshot"));
        assert!(!is_nonfilesystem_artifact_ref("artifacts/report.json"));
        assert!(!is_nonfilesystem_artifact_ref("/tmp/report.json"));
        assert!(!is_nonfilesystem_artifact_ref("notes:whatever"));
        assert!(!is_nonfilesystem_artifact_ref("evidence:claimed"));
    }
}
