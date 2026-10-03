use clap::Subcommand;

#[derive(Subcommand, Debug)]
pub enum WorkflowAction {
    /// Inspect audit state or request human-confirmed audit controls
    Audit {
        #[command(subcommand)]
        action: super::workflow_audit::AuditAction,
    },
    /// Create a workflow spec without executing it
    Plan(WorkflowPlanArgs),
    /// Create and execute a workflow
    Run(WorkflowRunArgs),
    /// Decompose a PRD with the fixed engine-native decomposition workflow
    Decompose(WorkflowDecomposeArgs),
    /// Print the embedded fixed decomposition runtime identity without launching a run
    #[command(name = "decomposition-identity")]
    DecompositionIdentity,
    /// Release a dead fixed run's task root without deleting evidence; disables its resume
    ReclaimTaskRoot(WorkflowReclaimTaskRootArgs),
    /// Show a workflow run status
    Status(WorkflowStatusArgs),
    /// Resume a paused or failed workflow
    Resume(WorkflowResumeArgs),
    /// Continue a workflow using the high-level recovery/resume surface
    Continue(WorkflowContinueArgs),
    /// Prepare repair from the first failed or blocked stage
    Repair(WorkflowRepairArgs),
    /// Pause a workflow
    Pause(WorkflowPauseArgs),
    /// Cancel a workflow
    Cancel(WorkflowCancelArgs),
    /// Approve a generated workflow once for this run
    #[command(name = "approve-run-once", alias = "approve-once")]
    ApproveRunOnce(WorkflowApproveRunOnceArgs),
    /// Always approve this workflow approval subject in this project
    #[command(name = "approve-always")]
    ApproveAlways(WorkflowApproveAlwaysArgs),
    /// Deny this workflow approval subject in this project and cancel the run
    #[command(name = "deny-workflow", alias = "deny")]
    DenyWorkflow(WorkflowDenyWorkflowArgs),
    /// Restart a single agent/item without rewinding the whole stage
    #[command(name = "restart-agent")]
    RestartAgent(WorkflowRestartAgentArgs),
    /// Restart an entire stage and its transitive dependents
    #[command(name = "restart-stage")]
    RestartStage(WorkflowRestartStageArgs),
    /// Restart a workflow task by task ID instead of internal stage ID
    #[command(name = "restart-task")]
    RestartTask(WorkflowRestartTaskArgs),
    /// Force-accept a failed stage with an audit rationale
    #[command(name = "force-accept", alias = "force-continue")]
    ForceAccept(WorkflowForceAcceptArgs),
    /// Save a sanitized reusable template
    Save(WorkflowSaveArgs),
    /// Evaluate topology gates over a task set, spec, or recorded graph
    ///
    /// Policy findings follow startup `workflow.gate_mode`: observe reports and
    /// exits zero; enforce reports and exits non-zero. Operational input errors
    /// always fail. This command never changes files or gates workflow admission.
    Lint(WorkflowLintArgs),
    /// Judge and freeze the acceptance contract beside a task set
    ///
    /// A check the judge does not accept is never published: it goes back to
    /// the check author with the judge's reason and counterexample, is
    /// re-judged, and after three attempts the freeze fails with a per-check
    /// report and writes nothing.
    ///
    /// With --reauthor, repairs an already frozen contract instead: only the
    /// named checks are re-authored and re-judged (same bound), every other
    /// entry stays byte-identical, and the contract, its lock, the task
    /// skeleton, the skeleton lock and the pin are republished in one atomic
    /// step, so no follow-up freeze-skeleton and no edit under the task
    /// directory is needed.
    ///
    /// Adopting a repaired contract in a PAUSED run: resume it
    /// (`archon workflow resume --live --yes <RUN_ID>`). Its acceptance stage is
    /// never replayed; every round re-reads the contract and the current pin
    /// from disk and verifies the chain, so the next round runs the repaired
    /// check. A running acceptance stage performs the same bounded repair
    /// in-round when it meets a check the judge did not accept. Each repair
    /// records a lineage link on the pin and files the chain it replaced by
    /// digest, so every round and the run-end observer can prove the current
    /// pin was reached from the run's launch pin by named re-authoring.
    FreezeAcceptance(WorkflowFreezeAcceptanceArgs),
    /// Validate and freeze the task skeleton before body writing
    FreezeSkeleton(WorkflowFreezeSkeletonArgs),
    /// Trusted child of the fixed decomposition: verify a frozen stage in place
    #[command(name = "verify-frozen-chain", hide = true)]
    VerifyFrozenChain(WorkflowVerifyFrozenChainArgs),
    /// Derive `.archon/project.json` from a decomposed task set
    ///
    /// Unions the environment keys the tasks declare into the project
    /// capability manifest the runtime merges into every task. Only ever adds:
    /// a project accumulates PRDs, and replacing the manifest would strip what
    /// an earlier decomposition put there. Tools are deliberately not carried
    /// — a declared tool must be invoked for a branch to be accepted, so a
    /// project-level tool obliges every task to run it.
    SyncCapabilities(WorkflowSyncCapabilitiesArgs),
    /// File surviving versions of a run's launch acceptance chain into the
    /// chain history, by digest
    ///
    /// Accepts only a contract or skeleton file whose blake3 digest the run's
    /// launch pin or the current pin's lineage names, checks every file before
    /// filing any, and writes nothing under the task directory.
    ImportChainHistory(WorkflowImportChainHistoryArgs),
    /// Re-run a finished run's observe-only run-end acceptance observation
    /// after it failed
    ///
    /// Refused unless the current pin is proven reached from the launch pin.
    /// Records the new outcome beside the kept failure; never changes the
    /// run's status.
    ObserveRunEnd(WorkflowObserveRunEndArgs),
    /// List workflow runs
    List,
}

#[cfg(test)]
mod audit_parse_tests {
    use crate::cli_args::Cli;
    use clap::Parser;

    #[test]
    fn repository_audit_operator_commands_parse_without_run_authority() {
        for args in [
            vec!["status", "wf-example"],
            vec![
                "extend-budget",
                "wf-example",
                "--extra-seconds",
                "7200",
                "--reason",
                "more time",
            ],
            vec![
                "set-budget",
                "wf-example",
                "--total-time-secs",
                "unlimited",
                "--reason",
                "large repository",
            ],
            vec![
                "reassess",
                "wf-example",
                "--finding",
                "file.txt",
                "--snapshot",
                "digest",
                "--reason",
                "counterevidence",
            ],
            vec![
                "waive",
                "wf-example",
                "--finding",
                "file.txt",
                "--snapshot",
                "digest",
                "--reason",
                "accepted exception",
            ],
        ] {
            let argv = [vec!["archon", "workflow", "audit"], args].concat();
            assert!(
                Cli::try_parse_from(&argv).is_ok(),
                "operator syntax rejected: {argv:?}"
            );
            let mut automated = argv.clone();
            automated.push("--yes");
            assert!(
                Cli::try_parse_from(automated).is_err(),
                "--yes must not authorize an audit mutation"
            );
        }
    }
}

#[path = "strategy_actions_workflow_args.rs"]
pub(super) mod strategy_actions_workflow_args;
pub use strategy_actions_workflow_args::*;
