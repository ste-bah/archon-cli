//! Which artifact refs name something the filesystem can be asked about.
//!
//! Existence validation is one leg of completion credit, so "does this
//! artifact exist" must have an answer for every ref a run can produce. Refs
//! naming something off the filesystem — an inline payload, an HTTP response,
//! an MCP resource — have no path to stat, and treating them as missing would
//! deny credit for work that was actually delivered.
//!
//! This sits beside the V2 result types rather than in the host because the
//! rule is a property of `WorkflowV2Result`'s artifact vocabulary, not of any
//! particular host's filesystem.

use super::result::WorkflowV2Result;

/// Schemes whose artifacts genuinely live outside the filesystem. An
/// allowlist, not a shape check: existence validation is one leg of
/// completion credit, so an arbitrary `whatever:` prefix must not bypass it.
const NONFILESYSTEM_SCHEMES: &[&str] = &["inline", "data", "http", "https", "mcp"];

pub fn is_nonfilesystem_artifact_ref(raw: &str) -> bool {
    let Some((scheme, _)) = raw.trim().split_once(':') else {
        return false;
    };
    NONFILESYSTEM_SCHEMES
        .iter()
        .any(|known| scheme.eq_ignore_ascii_case(known))
}

pub fn retain_filesystem_artifacts(result: &mut WorkflowV2Result) {
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
