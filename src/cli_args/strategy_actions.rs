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
}

#[derive(Subcommand, Debug, Clone, PartialEq, Eq)]
pub enum SandboxAction {
    /// Show configured sandbox backend and policy
    Status {
        /// Include compatibility and isolation details
        #[arg(long)]
        verbose: bool,
    },
    /// Explain how tools are routed through permission and sandbox gates
    Explain {
        /// Explain a specific backend instead of the configured backend
        #[arg(long)]
        backend: Option<String>,
    },
    /// Diagnose a sandbox backend without executing untrusted commands
    Doctor {
        /// Backend to diagnose: logical, docker, ssh, or openshell
        #[arg(long)]
        backend: Option<String>,
    },
    /// Validate sandbox config and report whether live execution is available
    Test {
        /// Backend to validate: logical, docker, ssh, or openshell
        #[arg(long)]
        backend: Option<String>,
    },
}

#[derive(Subcommand, Debug)]
pub enum GametheoryAction {
    /// Run full pipeline: classify → route → specialists → report
    Run {
        /// The strategic situation to analyze
        situation: String,
        /// Tier 1 classification only (skip routing and specialists)
        #[arg(long)]
        classify_only: bool,
        /// Path to gametheory spec YAML (searches known locations if omitted)
        #[arg(long, value_name = "PATH")]
        spec_path: Option<String>,
        /// Bind the run to an ingested document/knowledge pack
        #[arg(long, value_name = "PACK")]
        kb: Option<String>,
        /// Print per-agent gametheory memory recall counts
        #[arg(long)]
        debug_memory: bool,
        /// Stop specialist execution when estimated model spend reaches this USD cap
        #[arg(long, default_value_t = 20.0)]
        budget: f64,
        /// Maximum specialist concurrency requested for this run
        #[arg(long, default_value_t = 4)]
        max_concurrent: usize,
        /// Report style: executive, academic, or technical
        #[arg(long, default_value = "executive")]
        style: String,
        /// Enable Tier 11 specialists when policy.gametheory.enable_tier11 also allows it
        #[arg(long)]
        enable_tier11: bool,
    },
    /// List all persisted game-theory runs
    ListRuns,
    /// Show full details for a specific run
    Show {
        /// Run ID
        run_id: String,
    },
    /// Show status for one run, or status counts for all runs
    Status {
        /// Optional run ID
        run_id: Option<String>,
    },
    /// Inspect a run, specialist output, section, fingerprint, routing, or final report artifact
    Inspect {
        /// Artifact ID, e.g. gt-123, fingerprint:gt-123, specialist:gt-123:nash-equilibrium-finder
        artifact_id: String,
    },
    /// Inspect the Tier 1 fingerprint for a run
    InspectFingerprint {
        /// Run ID
        run_id: String,
    },
    /// Inspect the routing decision for a run
    InspectRouting {
        /// Run ID
        run_id: String,
    },
    /// Replay a run (re-evaluate routing from persisted fingerprint)
    Replay {
        /// Run ID
        run_id: String,
        /// Path to gametheory spec YAML (searches known locations if omitted)
        #[arg(long, value_name = "PATH")]
        spec_path: Option<String>,
        /// Re-run Tier 1 classification instead of preserving the stored fingerprint
        #[arg(long)]
        reclassify: bool,
        /// Re-run a single specialist using the stored Tier 1 fingerprint
        #[arg(long, value_name = "KEY")]
        rerun_specialist: Option<String>,
    },
    /// Resume an interrupted InProgress run from persisted checkpoints
    Resume {
        /// Run ID
        run_id: String,
        /// Path to gametheory spec YAML (searches known locations if omitted)
        #[arg(long, value_name = "PATH")]
        spec_path: Option<String>,
    },
    /// List curated game-theory agents
    ListAgents {
        /// Restrict output to a single tier
        #[arg(long, value_name = "N")]
        tier: Option<u8>,
    },
    /// List or ingest the known-fingerprint specimen library
    Specimens {
        /// Filter rows by axis=value, e.g. cooperation=cooperative
        #[arg(long, value_name = "AXIS=VALUE")]
        filter: Option<String>,
        /// Force re-ingest from the canonical markdown source
        #[arg(long)]
        ingest: bool,
    },
}

#[derive(Subcommand, Debug)]
pub enum CompletionAction {
    /// Run full completion-integrity check on a pipeline run
    Inspect {
        /// Run ID to inspect
        run_id: String,
        /// Task type for claim extraction (default: "pipeline-output")
        #[arg(long, default_value = "pipeline-output")]
        task_type: String,
    },
    /// List completion-sensitive claims for a run
    Claims {
        /// Run ID
        run_id: String,
    },
    /// List evidence records for a run
    Evidence {
        /// Run ID
        run_id: String,
    },
    /// List all false-completion incidents
    Incidents,
    /// Quick verify: run check and return pass/fail exit code
    Verify {
        /// Run ID to verify
        run_id: String,
        /// Task type for claim extraction
        #[arg(long, default_value = "pipeline-output")]
        task_type: String,
        /// Agent key responsible for the completion output
        #[arg(long, value_name = "KEY")]
        agent: Option<String>,
        /// Model responsible for the completion output
        #[arg(long, value_name = "NAME")]
        model: Option<String>,
        /// Workspace identifier for trust-score grouping
        #[arg(long, value_name = "ID")]
        workspace_id: Option<String>,
        /// Require at least one claim to exist (fail if none found)
        #[arg(long, default_value_t = false)]
        require_claims: bool,
    },
    /// Show persisted agent/model trust scores from completion verification history
    Trust {
        /// Filter to one agent key
        #[arg(long, value_name = "KEY")]
        agent: Option<String>,
        /// Filter to one model name
        #[arg(long, value_name = "NAME")]
        model: Option<String>,
    },
}

#[derive(Subcommand, Debug)]
pub enum TeamAction {
    /// Run a named team on a goal
    Run {
        /// Team name defined in config
        #[arg(long)]
        team: String,
        /// Goal for the team to accomplish
        goal: String,
    },
    /// List configured teams
    List,
}

#[derive(Subcommand, Debug)]
pub enum PipelineAction {
    /// Run the coding pipeline on a task
    Code {
        /// Task description for the coding pipeline
        task: String,
        /// Display agent sequence and estimated cost without executing
        #[arg(long)]
        dry_run: bool,
    },
    /// Run the research pipeline on a topic
    Research {
        /// Research topic
        topic: String,
        /// Display agent sequence and estimated cost without executing
        #[arg(long)]
        dry_run: bool,
    },
    /// Show status of a pipeline session
    Status {
        /// Session ID
        session_id: String,
    },
    /// Resume an interrupted pipeline session
    Resume {
        /// Session ID to resume
        session_id: String,
    },
    /// List all pipeline sessions
    List,
    /// Abort a running pipeline session
    Abort {
        /// Session ID to abort
        session_id: String,
    },
    /// Verify an audited built-in pipeline bundle
    Verify {
        /// Session ID to verify
        session_id: String,
        /// Also write verification/report.json into the bundle
        #[arg(long)]
        write_report: bool,
    },
    /// Inspect an audited built-in pipeline bundle
    Inspect {
        /// Session ID to inspect
        session_id: String,
    },
    /// Export verified built-in pipeline traces
    #[command(name = "export-traces")]
    ExportTraces {
        /// Session ID to export
        session_id: String,
        /// Export format; currently only jsonl is supported
        #[arg(long, default_value = "jsonl")]
        format: String,
        /// Output file path. Omit to print to stdout.
        #[arg(long)]
        out: Option<std::path::PathBuf>,
        /// Export even if the bundle verifier reports errors
        #[arg(long)]
        include_unverified: bool,
    },
    /// Run a declarative pipeline from a spec file
    #[command(name = "run")]
    Run {
        /// Path to pipeline spec file (YAML or JSON)
        file: std::path::PathBuf,
        /// Override format auto-detection (yaml or json)
        #[arg(long)]
        format: Option<String>,
        /// Return immediately after submission (don't poll for completion)
        #[arg(long)]
        detach: bool,
    },
    /// Cancel a running declarative pipeline
    #[command(name = "cancel")]
    Cancel {
        /// Pipeline run ID (UUID)
        id: String,
    },
}
