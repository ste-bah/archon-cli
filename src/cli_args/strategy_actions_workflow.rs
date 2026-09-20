use clap::Subcommand;

#[derive(Subcommand, Debug)]
pub enum WorkflowAction {
    /// Inspect audit state or request human-confirmed audit controls
    Audit {
        #[command(subcommand)]
        action: super::workflow_audit::AuditAction,
    },
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
    /// Decompose a PRD with the fixed engine-native decomposition workflow
    Decompose {
        /// PRD file to decompose
        #[arg(long, value_name = "PATH")]
        prd: std::path::PathBuf,
        /// Destination task-set directory
        #[arg(long, value_name = "DIR")]
        tasks: std::path::PathBuf,
        /// Code repository the authors are grounded in (a git checkout); overrides
        /// [workflow] repository_root and [workflow.acceptance_execution].repository
        #[arg(long, value_name = "PATH")]
        repository: Option<std::path::PathBuf>,
        /// Approve this fixed live decomposition for non-interactive execution
        #[arg(long)]
        yes: bool,
    },
    /// Print the embedded fixed decomposition runtime identity without launching a run
    #[command(name = "decomposition-identity")]
    DecompositionIdentity,
    /// Release a dead fixed run's task root without deleting evidence; disables its resume
    ReclaimTaskRoot {
        run_id: String,
        /// Confirm permanent release of this run's task-root ownership
        #[arg(long)]
        yes: bool,
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
    /// Evaluate topology gates over a task set, spec, or recorded graph
    ///
    /// Policy findings follow startup `workflow.gate_mode`: observe reports and
    /// exits zero; enforce reports and exits non-zero. Operational input errors
    /// always fail. This command never changes files or gates workflow admission.
    Lint {
        /// Consume one candidate task body from stdin into run-owned staging
        #[arg(long, hide = true)]
        candidate_stdin: bool,
        /// Run-owned child staging root
        #[arg(long, value_name = "DIR", hide = true)]
        staging_root: Option<std::path::PathBuf>,
        /// Typed gate-envelope side-channel
        #[arg(long, value_name = "PATH", hide = true)]
        gate_envelope: Option<std::path::PathBuf>,
        /// Parent-owned canonical host-call identity
        #[arg(long, value_name = "ID", hide = true)]
        call_id: Option<String>,
        /// Exactly one decomposed-PRD TASK-*.md file to lint
        #[arg(long = "task-file", value_name = "PATH")]
        task_file: Option<std::path::PathBuf>,
        /// Directory of decomposed-PRD TASK-*.md files to lint
        #[arg(long, value_name = "DIR")]
        tasks: Option<std::path::PathBuf>,
        /// Workflow spec file to lint
        #[arg(long = "spec-file", value_name = "PATH")]
        spec_file: Option<std::path::PathBuf>,
        /// Recorded graph id under .archon/topology to lint
        #[arg(long, value_name = "ID")]
        graph: Option<String>,
        /// Also ask the critic model whether each claimed PRD obligation is
        /// necessarily true once its claiming tasks pass (costs tokens; the
        /// decomposition's set gate runs it unconditionally)
        #[arg(long)]
        fidelity: bool,
        /// Waive a fidelity finding for this obligation id (repeatable);
        /// recorded verbatim in the task set's freeze pin
        #[arg(long = "waive-obligation", value_name = "ID")]
        waive_obligation: Vec<String>,
        /// The operator's reason for every --waive-obligation given
        #[arg(long = "waive-reason", value_name = "TEXT")]
        waive_reason: Option<String>,
    },
    /// Judge and freeze the acceptance contract beside a task set
    FreezeAcceptance {
        #[arg(long, value_name = "DIR")]
        tasks: std::path::PathBuf,
        #[arg(long, value_name = "PATH")]
        prd: std::path::PathBuf,
        /// Consume the candidate acceptance contract from stdin
        #[arg(long, hide = true)]
        candidate_stdin: bool,
        /// Run-owned child staging root
        #[arg(long, value_name = "DIR", hide = true)]
        staging_root: Option<std::path::PathBuf>,
        /// Typed gate-envelope side-channel
        #[arg(long, value_name = "PATH", hide = true)]
        gate_envelope: Option<std::path::PathBuf>,
        /// Parent-owned canonical host-call identity
        #[arg(long, value_name = "ID", hide = true)]
        call_id: Option<String>,
    },
    /// Validate and freeze the task skeleton before body writing
    FreezeSkeleton {
        #[arg(long, value_name = "DIR")]
        tasks: std::path::PathBuf,
        #[arg(long, value_name = "PATH")]
        prd: std::path::PathBuf,
        /// Consume the candidate task skeleton from stdin
        #[arg(long, hide = true)]
        candidate_stdin: bool,
        /// Run-owned child staging root
        #[arg(long, value_name = "DIR", hide = true)]
        staging_root: Option<std::path::PathBuf>,
        /// Typed gate-envelope side-channel
        #[arg(long, value_name = "PATH", hide = true)]
        gate_envelope: Option<std::path::PathBuf>,
        /// Parent-owned canonical host-call identity
        #[arg(long, value_name = "ID", hide = true)]
        call_id: Option<String>,
    },
    /// Trusted child of the fixed decomposition: verify a frozen stage in place
    #[command(name = "verify-frozen-chain", hide = true)]
    VerifyFrozenChain {
        /// Which frozen stage to verify: acceptance or skeleton
        #[arg(long, value_name = "STAGE")]
        stage: String,
        #[arg(long, value_name = "DIR")]
        tasks: std::path::PathBuf,
        #[arg(long, value_name = "PATH")]
        prd: std::path::PathBuf,
        /// Typed gate-envelope side-channel
        #[arg(long, value_name = "PATH")]
        gate_envelope: Option<std::path::PathBuf>,
        /// Parent-owned canonical host-call identity
        #[arg(long, value_name = "ID")]
        call_id: Option<String>,
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

#[cfg(test)]
mod audit_parse_tests {
    use clap::Parser;
    use crate::cli_args::Cli;

    #[test]
    fn repository_audit_operator_commands_parse_without_run_authority() {
        for args in [
            vec!["status", "wf-example"],
            vec!["extend-budget", "wf-example", "--extra-seconds", "7200", "--reason", "more time"],
            vec!["set-budget", "wf-example", "--total-time-secs", "unlimited", "--reason", "large repository"],
            vec!["reassess", "wf-example", "--finding", "file.txt", "--snapshot", "digest", "--reason", "counterevidence"],
            vec!["waive", "wf-example", "--finding", "file.txt", "--snapshot", "digest", "--reason", "accepted exception"],
        ] {
            let argv = [vec!["archon", "workflow", "audit"], args].concat();
            assert!(Cli::try_parse_from(&argv).is_ok(), "operator syntax rejected: {argv:?}");
            let mut automated = argv.clone();
            automated.push("--yes");
            assert!(Cli::try_parse_from(automated).is_err(), "--yes must not authorize an audit mutation");
        }
    }
}
