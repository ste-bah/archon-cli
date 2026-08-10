use clap::Subcommand;

#[path = "strategy_actions_providers.rs"]
mod strategy_actions_providers;
pub use strategy_actions_providers::*;

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
        /// Explain routing for a specific tool, e.g. Bash
        #[arg(long)]
        tool: Option<String>,
        /// Optional command preview for shell routing explanations
        #[arg(long)]
        command: Option<String>,
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
    /// List audited sandbox sessions from the Cozo learning store
    Sessions {
        /// Filter by sandbox session status, e.g. configured
        #[arg(long)]
        status: Option<String>,
        /// Filter by agent type
        #[arg(long)]
        agent: Option<String>,
        /// Maximum rows to show
        #[arg(long, default_value_t = 20)]
        limit: usize,
        /// Output session rows as JSON
        #[arg(long)]
        json: bool,
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
        /// Continue past a critical quality-gate failure and audit the override
        #[arg(long)]
        force_quality_gate: bool,
    },
    /// Rewind an audited pipeline so resume re-runs from a chosen agent
    Rewind {
        /// Session ID to rewind
        session_id: String,
        /// Re-run this agent on next resume
        #[arg(long, conflicts_with_all = ["to_ordinal", "keep_agents"])]
        to_agent: Option<String>,
        /// Re-run this ordinal on next resume
        #[arg(long, conflicts_with_all = ["to_agent", "keep_agents"])]
        to_ordinal: Option<usize>,
        /// Keep exactly this many completed agents
        #[arg(long, conflicts_with_all = ["to_agent", "to_ordinal"])]
        keep_agents: Option<usize>,
        /// Audited reason for the rewind
        #[arg(long, default_value = "operator requested pipeline rewind")]
        reason: String,
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

#[derive(Subcommand, Debug)]
pub enum WorkflowAction {
    /// Create a workflow spec without executing it
    Plan {
        /// Validate an existing workflow spec file instead of planning from text
        #[arg(long, value_name = "PATH")]
        spec_file: Option<std::path::PathBuf>,
        /// Use the legacy decomposed lifecycle instead of the default v3 authored-script lifecycle
        #[arg(long)]
        decomposed: bool,
        /// Use the configured provider for planning instead of deterministic smoke mode
        #[arg(long)]
        live: bool,
        /// Natural-language task
        task: Vec<String>,
    },
    /// Create and execute a workflow
    Run {
        /// Execute an existing workflow spec file instead of planning from text
        #[arg(long, value_name = "PATH")]
        spec_file: Option<std::path::PathBuf>,
        /// Execute a saved project workflow template
        #[arg(long = "from-template", value_name = "NAME")]
        from_template: Option<String>,
        /// Resume a prior generated V2 run and reuse its accepted/noop calls
        #[arg(long = "resume-from", value_name = "RUN_ID")]
        resume_from: Option<String>,
        /// Use the legacy decomposed lifecycle instead of the default v3 authored-script lifecycle
        #[arg(long)]
        decomposed: bool,
        /// Use the configured provider for live stage agents
        #[arg(long)]
        live: bool,
        /// Approve this generated/saved workflow for a non-interactive live run
        #[arg(long)]
        yes: bool,
        /// Natural-language task
        task: Vec<String>,
    },
    /// Show a workflow run status
    Status {
        /// Workflow run ID
        run_id: String,
    },
    /// Resume a paused or failed workflow
    Resume {
        /// Use the configured provider for live stage agents
        #[arg(long)]
        live: bool,
        /// Approve this resume for non-interactive live execution
        #[arg(long)]
        yes: bool,
        /// Workflow run ID
        run_id: String,
    },
    /// Continue a workflow using the high-level recovery/resume surface
    Continue {
        /// Use the configured provider for live stage agents
        #[arg(long)]
        live: bool,
        /// Approve this continue for non-interactive live execution
        #[arg(long)]
        yes: bool,
        /// Workflow run ID
        run_id: String,
    },
    /// Prepare repair from the first failed or blocked stage
    Repair {
        /// Workflow run ID
        run_id: String,
    },
    /// Pause a workflow
    Pause {
        /// Workflow run ID
        run_id: String,
    },
    /// Cancel a workflow
    Cancel {
        /// Workflow run ID
        run_id: String,
    },
    /// Approve a generated workflow once for this run
    #[command(name = "approve-run-once", alias = "approve-once")]
    ApproveRunOnce {
        /// Workflow run ID
        run_id: String,
    },
    /// Always approve this workflow approval subject in this project
    #[command(name = "approve-always")]
    ApproveAlways {
        /// Workflow run ID
        run_id: String,
    },
    /// Deny this workflow approval subject in this project and cancel the run
    #[command(name = "deny-workflow", alias = "deny")]
    DenyWorkflow {
        /// Workflow run ID
        run_id: String,
    },
    /// Restart a single agent/item without rewinding the whole stage
    #[command(name = "restart-agent")]
    RestartAgent {
        /// Workflow run ID
        run_id: String,
        /// Stage ID to rewind
        stage_id: String,
        /// Optional fan-out item id; when set, only this item is rewound
        item: Option<String>,
    },
    /// Restart an entire stage and its transitive dependents
    #[command(name = "restart-stage")]
    RestartStage {
        /// Workflow run ID
        run_id: String,
        /// Stage ID to rewind
        stage_id: String,
    },
    /// Restart a workflow task by task ID instead of internal stage ID
    #[command(name = "restart-task")]
    RestartTask {
        /// Workflow run ID
        run_id: String,
        /// Canonical task ID to restart
        task_id: String,
    },
    /// Force-accept a failed stage with an audit rationale
    #[command(name = "force-accept", alias = "force-continue")]
    ForceAccept {
        /// Workflow run ID
        run_id: String,
        /// Stage ID to force accept
        stage_id: String,
        /// Human rationale written to the audit log
        rationale: Vec<String>,
    },
    /// Save a sanitized reusable template
    Save {
        /// Workflow run ID
        run_id: String,
        /// Template name
        name: String,
    },
    /// Run the advisory topology lints over a task set, spec, or recorded graph
    ///
    /// Reports only. No lint here can fail a run or change a file.
    Lint {
        /// Directory of decomposed-PRD TASK-*.md files to lint
        #[arg(long, value_name = "DIR")]
        tasks: Option<std::path::PathBuf>,
        /// Workflow spec file to lint
        #[arg(long = "spec-file", value_name = "PATH")]
        spec_file: Option<std::path::PathBuf>,
        /// Recorded graph id under .archon/topology to lint
        #[arg(long, value_name = "ID")]
        graph: Option<String>,
    },
    /// Derive `.archon/project.json` from a decomposed task set
    ///
    /// Unions the environment keys the tasks declare into the project
    /// capability manifest the runtime merges into every task. Only ever adds:
    /// a project accumulates PRDs, and replacing the manifest would strip what
    /// an earlier decomposition put there. Tools are deliberately not carried
    /// — a declared tool must be invoked for a branch to be accepted, so a
    /// project-level tool obliges every task to run it.
    SyncCapabilities {
        /// Directory of decomposed-PRD TASK-*.md files to derive from
        #[arg(long, value_name = "DIR")]
        tasks: std::path::PathBuf,
        /// Report what would change without writing the manifest
        #[arg(long)]
        dry_run: bool,
    },
    /// List workflow runs
    List,
}
