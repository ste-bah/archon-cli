//! Argument structs for `WorkflowAction` variants. Each struct gets its own
//! clap `augment_args` frame, so the frames do not stack (#233).

use clap::Args;

#[derive(Args, Debug)]
pub struct WorkflowPlanArgs {
    /// Validate an existing workflow spec file instead of planning from text
    #[arg(long, value_name = "PATH")]
    pub spec_file: Option<std::path::PathBuf>,
    /// Use the legacy decomposed lifecycle instead of the default v3 authored-script lifecycle
    #[arg(long)]
    pub decomposed: bool,
    /// Use the configured provider for planning instead of deterministic smoke mode
    #[arg(long)]
    pub live: bool,
    /// Natural-language task
    pub task: Vec<String>,
}

#[derive(Args, Debug)]
pub struct WorkflowRunArgs {
    /// Execute an existing workflow spec file instead of planning from text
    #[arg(long, value_name = "PATH")]
    pub spec_file: Option<std::path::PathBuf>,
    /// Execute a saved project workflow template
    #[arg(long = "from-template", value_name = "NAME")]
    pub from_template: Option<String>,
    /// Resume a prior generated V2 run and reuse its accepted/noop calls
    #[arg(long = "resume-from", value_name = "RUN_ID")]
    pub resume_from: Option<String>,
    /// Use the legacy decomposed lifecycle instead of the default v3 authored-script lifecycle
    #[arg(long)]
    pub decomposed: bool,
    /// Use the configured provider for live stage agents
    #[arg(long)]
    pub live: bool,
    /// Approve this generated/saved workflow for a non-interactive live run
    #[arg(long)]
    pub yes: bool,
    /// Natural-language task
    pub task: Vec<String>,
}

#[derive(Args, Debug)]
pub struct WorkflowDecomposeArgs {
    /// PRD file to decompose
    #[arg(long, value_name = "PATH")]
    pub prd: std::path::PathBuf,
    /// Destination task-set directory
    #[arg(long, value_name = "DIR")]
    pub tasks: std::path::PathBuf,
    /// Code repository the authors are grounded in (a git checkout); overrides
    /// [workflow] repository_root and [workflow.acceptance_execution].repository
    #[arg(long, value_name = "PATH")]
    pub repository: Option<std::path::PathBuf>,
    /// Approve this fixed live decomposition for non-interactive execution
    #[arg(long)]
    pub yes: bool,
}

#[derive(Args, Debug)]
pub struct WorkflowReclaimTaskRootArgs {
    pub run_id: String,
    /// Confirm permanent release of this run's task-root ownership
    #[arg(long)]
    pub yes: bool,
}

#[derive(Args, Debug)]
pub struct WorkflowStatusArgs {
    /// Workflow run ID
    pub run_id: String,
}

#[derive(Args, Debug)]
pub struct WorkflowResumeArgs {
    /// Use the configured provider for live stage agents
    #[arg(long)]
    pub live: bool,
    /// Approve this resume for non-interactive live execution
    #[arg(long)]
    pub yes: bool,
    /// Inspect the fixed resume plan without acquiring ownership or writing run files
    #[arg(long)]
    pub dry_run: bool,
    /// Workflow run ID
    pub run_id: String,
}

#[derive(Args, Debug)]
pub struct WorkflowContinueArgs {
    /// Use the configured provider for live stage agents
    #[arg(long)]
    pub live: bool,
    /// Approve this continue for non-interactive live execution
    #[arg(long)]
    pub yes: bool,
    /// Workflow run ID
    pub run_id: String,
}

#[derive(Args, Debug)]
pub struct WorkflowRepairArgs {
    /// Workflow run ID
    pub run_id: String,
}

#[derive(Args, Debug)]
pub struct WorkflowPauseArgs {
    /// Workflow run ID
    pub run_id: String,
}

#[derive(Args, Debug)]
pub struct WorkflowCancelArgs {
    /// Workflow run ID
    pub run_id: String,
}

#[derive(Args, Debug)]
pub struct WorkflowApproveRunOnceArgs {
    /// Workflow run ID
    pub run_id: String,
}

#[derive(Args, Debug)]
pub struct WorkflowApproveAlwaysArgs {
    /// Workflow run ID
    pub run_id: String,
}

#[derive(Args, Debug)]
pub struct WorkflowDenyWorkflowArgs {
    /// Workflow run ID
    pub run_id: String,
}

#[derive(Args, Debug)]
pub struct WorkflowRestartAgentArgs {
    /// Workflow run ID
    pub run_id: String,
    /// Stage ID to rewind
    pub stage_id: String,
    /// Optional fan-out item id; when set, only this item is rewound
    pub item: Option<String>,
}

#[derive(Args, Debug)]
pub struct WorkflowRestartStageArgs {
    /// Workflow run ID
    pub run_id: String,
    /// Stage ID to rewind
    pub stage_id: String,
}

#[derive(Args, Debug)]
pub struct WorkflowRestartTaskArgs {
    /// Workflow run ID
    pub run_id: String,
    /// Canonical task ID to restart
    pub task_id: String,
}

#[derive(Args, Debug)]
pub struct WorkflowForceAcceptArgs {
    /// Workflow run ID
    pub run_id: String,
    /// Stage ID to force accept
    pub stage_id: String,
    /// Human rationale written to the audit log
    pub rationale: Vec<String>,
}

#[derive(Args, Debug)]
pub struct WorkflowSaveArgs {
    /// Workflow run ID
    pub run_id: String,
    /// Template name
    pub name: String,
}

#[derive(Args, Debug)]
pub struct WorkflowLintArgs {
    /// Consume one candidate task body from stdin into run-owned staging
    #[arg(long, hide = true)]
    pub candidate_stdin: bool,
    /// Run-owned child staging root
    #[arg(long, value_name = "DIR", hide = true)]
    pub staging_root: Option<std::path::PathBuf>,
    /// Typed gate-envelope side-channel
    #[arg(long, value_name = "PATH", hide = true)]
    pub gate_envelope: Option<std::path::PathBuf>,
    /// Parent-owned canonical host-call identity
    #[arg(long, value_name = "ID", hide = true)]
    pub call_id: Option<String>,
    /// Exactly one decomposed-PRD TASK-*.md file to lint
    #[arg(long = "task-file", value_name = "PATH")]
    pub task_file: Option<std::path::PathBuf>,
    /// Directory of decomposed-PRD TASK-*.md files to lint
    #[arg(long, value_name = "DIR")]
    pub tasks: Option<std::path::PathBuf>,
    /// Workflow spec file to lint
    #[arg(long = "spec-file", value_name = "PATH")]
    pub spec_file: Option<std::path::PathBuf>,
    /// Recorded graph id under .archon/topology to lint
    #[arg(long, value_name = "ID")]
    pub graph: Option<String>,
    /// Also ask the critic model whether each claimed PRD obligation is
    /// necessarily true once its claiming tasks pass (costs tokens; the
    /// decomposition's set gate runs it unconditionally)
    #[arg(long)]
    pub fidelity: bool,
    /// Waive a fidelity finding for this obligation id (repeatable);
    /// recorded verbatim in the task set's freeze pin
    #[arg(long = "waive-obligation", value_name = "ID")]
    pub waive_obligation: Vec<String>,
    /// The operator's reason for every --waive-obligation given
    #[arg(long = "waive-reason", value_name = "TEXT")]
    pub waive_reason: Option<String>,
}

#[derive(Args, Debug)]
pub struct WorkflowFreezeAcceptanceArgs {
    #[arg(long, value_name = "DIR")]
    pub tasks: std::path::PathBuf,
    #[arg(long, value_name = "PATH")]
    pub prd: std::path::PathBuf,
    /// Re-author and re-judge only this frozen check (repeatable); every
    /// other entry is kept byte-identical and the whole chain, skeleton
    /// lock included, is republished atomically. Refused when the id is
    /// not in the frozen contract.
    #[arg(long = "reauthor", value_name = "CHECK_ID")]
    pub reauthor: Vec<String>,
    /// Consume the candidate acceptance contract from stdin
    #[arg(long, hide = true)]
    pub candidate_stdin: bool,
    /// Run-owned child staging root
    #[arg(long, value_name = "DIR", hide = true)]
    pub staging_root: Option<std::path::PathBuf>,
    /// Typed gate-envelope side-channel
    #[arg(long, value_name = "PATH", hide = true)]
    pub gate_envelope: Option<std::path::PathBuf>,
    /// Parent-owned canonical host-call identity
    #[arg(long, value_name = "ID", hide = true)]
    pub call_id: Option<String>,
}

#[derive(Args, Debug)]
pub struct WorkflowFreezeSkeletonArgs {
    #[arg(long, value_name = "DIR")]
    pub tasks: std::path::PathBuf,
    #[arg(long, value_name = "PATH")]
    pub prd: std::path::PathBuf,
    /// Consume the candidate task skeleton from stdin
    #[arg(long, hide = true)]
    pub candidate_stdin: bool,
    /// Run-owned child staging root
    #[arg(long, value_name = "DIR", hide = true)]
    pub staging_root: Option<std::path::PathBuf>,
    /// Typed gate-envelope side-channel
    #[arg(long, value_name = "PATH", hide = true)]
    pub gate_envelope: Option<std::path::PathBuf>,
    /// Parent-owned canonical host-call identity
    #[arg(long, value_name = "ID", hide = true)]
    pub call_id: Option<String>,
}

#[derive(Args, Debug)]
pub struct WorkflowVerifyFrozenChainArgs {
    /// Which frozen stage to verify: acceptance or skeleton
    #[arg(long, value_name = "STAGE")]
    pub stage: String,
    #[arg(long, value_name = "DIR")]
    pub tasks: std::path::PathBuf,
    #[arg(long, value_name = "PATH")]
    pub prd: std::path::PathBuf,
    /// Typed gate-envelope side-channel
    #[arg(long, value_name = "PATH")]
    pub gate_envelope: Option<std::path::PathBuf>,
    /// Parent-owned canonical host-call identity
    #[arg(long, value_name = "ID")]
    pub call_id: Option<String>,
}

#[derive(Args, Debug)]
pub struct WorkflowSyncCapabilitiesArgs {
    /// Directory of decomposed-PRD TASK-*.md files to derive from
    #[arg(long, value_name = "DIR")]
    pub tasks: std::path::PathBuf,
    /// Report what would change without writing the manifest
    #[arg(long)]
    pub dry_run: bool,
}

#[derive(Args, Debug)]
pub struct WorkflowImportChainHistoryArgs {
    /// Workflow run ID
    pub run_id: String,
    /// A contract or skeleton file to file (repeatable)
    #[arg(long = "from", value_name = "FILE", required = true)]
    pub from: Vec<std::path::PathBuf>,
}

#[derive(Args, Debug)]
pub struct WorkflowObserveRunEndArgs {
    /// Workflow run ID
    pub run_id: String,
}
