use clap::Subcommand;

#[derive(Subcommand, Debug, Clone, PartialEq, Eq)]
pub enum PermissionsAction {
    /// Summarize Cozo-backed permission runtime events
    Audit {
        /// Output the audit as JSON
        #[arg(long)]
        json: bool,
    },
    /// List persisted permission denials
    Denials {
        /// Restrict output to one agent type
        #[arg(long)]
        agent: Option<String>,
        /// Maximum denied events to display
        #[arg(long, default_value_t = 20)]
        limit: usize,
        /// Output matching denials as JSON
        #[arg(long)]
        json: bool,
    },
}
