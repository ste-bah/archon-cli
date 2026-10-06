use super::TraceOptions;
use std::path::PathBuf;

impl TraceOptions {
    /// Defaults chosen so a bare `--prd/--tasks` run costs no queries at all and
    /// still answers the coverage question.
    pub(crate) fn new(prd: PathBuf, tasks: PathBuf) -> Self {
        Self {
            prd,
            tasks,
            graph: None,
            evidence: None,
            leann_db: None,
            persist: None,
            falsify: false,
            check_policy: None,
            json: false,
            limit_per_scope: 3,
            max_scopes: 8,
            // Both entry points overwrite this from `[memory]`; what is left
            // reaching the default is a test that built its own index with the
            // same default, which is consistent by construction.
            embedding: archon_memory::embedding::EmbeddingConfig::default(),
        }
    }
}

pub(super) fn from_config(
    config: &archon_core::config::ArchonConfig,
) -> anyhow::Result<Option<archon_workflow::acceptance_check_environment::CheckPolicy>> {
    crate::command::acceptance_check_policy::from_config(config)
}
