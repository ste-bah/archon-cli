use clap::Subcommand;

#[derive(Subcommand, Debug)]
pub enum RequirementsAction {
    /// Trace PRD requirements to code, with a proof ladder
    ///
    /// Reports per requirement: its proof level, its anchors, and — below
    /// `Exercised` — exactly what is missing. An unproven requirement is a
    /// declared residual gap with fail-closed behaviour (PRD §32), not a
    /// failure and not a pass, so the exit status is success either way.
    ///
    /// Read-only. It never indexes: `archon-leann` holds the Cozo write lock
    /// across an entire multi_transaction, so the index must be built out of
    /// band before tracing.
    Trace {
        /// PRD markdown to extract requirement IDs from
        #[arg(long, value_name = "PATH")]
        prd: std::path::PathBuf,
        /// Directory of decomposed-PRD TASK-*.md files
        #[arg(long, value_name = "DIR")]
        tasks: std::path::PathBuf,
        /// Recorded graph id under .archon/topology, for FileRead evidence
        ///
        /// Without it no anchor can reach `Exercised`, because there is no
        /// trace to show a verifier reading the anchored file.
        #[arg(long, value_name = "ID")]
        graph: Option<String>,
        /// A run's final report JSON, for `commands_run` evidence
        #[arg(long, value_name = "PATH")]
        evidence: Option<std::path::PathBuf>,
        /// Existing LEANN code index to anchor against (never created here)
        #[arg(long = "leann-db", value_name = "PATH")]
        leann_db: Option<std::path::PathBuf>,
        /// Persist requirement entities and anchored edges into this store
        #[arg(long, value_name = "PATH")]
        persist: Option<std::path::PathBuf>,
        /// Emit the report model as JSON
        #[arg(long)]
        json: bool,
        /// Index hits requested per declared path scope
        #[arg(long, value_name = "N", default_value = "3")]
        limit_per_scope: usize,
        /// Declared path scopes searched per task, capping the query budget
        #[arg(long, value_name = "N", default_value = "8")]
        max_scopes: usize,
    },
}
