use clap::Subcommand;

#[derive(Subcommand, Debug, Clone, PartialEq, Eq)]
pub enum ProvidersAction {
    /// Show provider registry entries
    List,
    /// Show Archon surface support by provider/auth mode
    Capabilities,
    /// Show provider-neutral runtime status from local configuration
    Status {
        /// Restrict output to one provider id
        #[arg(long)]
        provider: Option<String>,
        /// Output the status snapshot as JSON
        #[arg(long)]
        json: bool,
        /// Run opt-in live endpoint reachability checks
        #[arg(long)]
        live: bool,
    },
    /// Summarize provider health from status and persisted runtime events
    Report {
        /// Restrict output to one provider id
        #[arg(long)]
        provider: Option<String>,
        /// Output the report as JSON
        #[arg(long)]
        json: bool,
    },
    /// Show persisted provider rate-limit windows
    Limits {
        /// Restrict output to one provider id
        #[arg(long)]
        provider: Option<String>,
    },
    /// Inspect persisted provider auth profiles
    Profiles {
        #[command(subcommand)]
        action: ProviderProfilesAction,
    },
    /// Diagnose provider/auth configuration
    Doctor {
        /// Run opt-in live endpoint reachability checks
        #[arg(long)]
        live: bool,
    },
}

#[derive(Subcommand, Debug, Clone, PartialEq, Eq)]
pub enum ProviderProfilesAction {
    /// Import current local/env credentials into the Cozo auth profile store
    Import,
    /// List persisted auth profiles
    List {
        /// Restrict output to one provider id
        #[arg(long)]
        provider: Option<String>,
    },
    /// Inspect one persisted auth profile
    Inspect {
        /// Profile id to inspect
        profile_id: String,
    },
    /// Clear a profile cooldown marker
    CooldownClear {
        /// Profile id to update
        profile_id: String,
    },
    /// Show ordered profile selection and skip reasons
    Select {
        /// Provider id to select for
        provider: String,
        /// Restrict to one or more auth kinds
        #[arg(long = "auth-kind")]
        auth_kinds: Vec<String>,
        /// Prefer this profile id when it is healthy
        #[arg(long)]
        preferred: Option<String>,
    },
}
