use archon_workflow::WorkflowV2Result;

pub(super) fn is_nonfilesystem_artifact_ref(raw: &str) -> bool {
    let Some((scheme, _)) = raw.trim().split_once(':') else {
        return false;
    };
    let mut chars = scheme.chars();
    chars.next().is_some_and(|first| first.is_ascii_lowercase())
        && chars.all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit())
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
    fn recognizes_only_nonfilesystem_scheme_refs() {
        assert!(is_nonfilesystem_artifact_ref("inline:data.items"));
        assert!(is_nonfilesystem_artifact_ref("https://example.test/report"));
        assert!(!is_nonfilesystem_artifact_ref("artifacts/report.json"));
        assert!(!is_nonfilesystem_artifact_ref("/tmp/report.json"));
        assert!(!is_nonfilesystem_artifact_ref("Upper:value"));
    }
}
