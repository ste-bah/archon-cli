//! Host evidence distinguishing project delivery from repository patch work.
use super::*;
use crate::v2::project_artifact_prompt::declared_project_artifacts;

pub(super) struct ArtifactDelivery {
    paths: Vec<(String, PathBuf)>,
    before: BTreeMap<String, Option<String>>,
}
fn digest(path: &Path) -> Option<String> {
    let meta = path.symlink_metadata().ok()?;
    if !meta.is_file() {
        return None;
    }
    std::fs::read(path)
        .ok()
        .map(|bytes| crate::task_set_contract::content_digest(&bytes))
}
impl ArtifactDelivery {
    pub fn capture(prepared: &PreparedWorktreeBranch, store: &WorkflowV2ResultStore) -> Self {
        let context = crate::project_artifact_context_from_v2_root(store.root());
        let declared = declared_project_artifacts(
            &prepared.branch.input,
            &prepared.branch.call.options.required_artifacts,
            &context,
        );
        let paths = declared
            .entries
            .into_iter()
            .map(|(raw, absolute)| (raw, PathBuf::from(absolute)))
            .collect::<Vec<_>>();
        let before = paths
            .iter()
            .map(|(raw, path)| (raw.clone(), digest(path)))
            .collect();
        Self { paths, before }
    }
    pub fn stamp(&self, result: &mut WorkflowV2Result, repository_changed: bool) {
        if !result.data.is_object() {
            result.data = serde_json::json!({});
        }
        if self.paths.is_empty() {
            result.data["delivery"] = serde_json::json!({"kind":if repository_changed{"repository_patch"}else{"no_repository_change"},"repository_changed":repository_changed});
            return;
        }
        let after = self
            .paths
            .iter()
            .map(|(raw, path)| (raw.clone(), digest(path)))
            .collect::<BTreeMap<_, _>>();
        let changed = after
            .iter()
            .filter(|(key, value)| self.before.get(*key) != Some(value))
            .map(|(key, _)| key.clone())
            .collect::<Vec<_>>();
        result.data["delivery"] = serde_json::json!({"kind":"project_artifact","repository_changed":repository_changed,
            "changed_artifact_paths":changed,"before":self.before,"after":after});
        // Required-artifact validation remains authoritative; a receipt never
        // promotes failed output or claims a nonexistent file was delivered.
        if result.status == WorkflowV2Status::Accepted
            && !repository_changed
            && changed.is_empty()
            && self
                .paths
                .iter()
                .all(|(raw, _)| after.get(raw).is_some_and(Option::is_some))
        {
            result.status = WorkflowV2Status::Noop;
        }
    }
}
