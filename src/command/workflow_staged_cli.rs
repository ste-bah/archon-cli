//! Hidden trusted-child composition for fixed-decomposition lint capabilities.

use std::io::Read;
use std::path::Path;

use anyhow::{Result, anyhow};
use archon_workflow::WorkflowLlmClientFactory;

/// The decomposition's set gate: the point at which a task set with bodies is
/// accepted, so the obligation fidelity audit runs here unconditionally.
///
/// The critic client comes from the configured provider only — the child runs
/// under the freeze provider environment the parent resolved, never an
/// ambient one — and a client that cannot be built reaches the envelope as an
/// operational error rather than a pass: a provider outage must stop the
/// freeze, not certify it. Waivers are the ones an operator recorded in the
/// task set's pin; the catalog argv is fixed, so there is no flag to pass here.
#[allow(clippy::too_many_arguments)]
pub(super) async fn handle_staged_task_set_lint(
    cwd: &Path,
    task_file: Option<&Path>,
    tasks: Option<&Path>,
    spec_file: Option<&Path>,
    graph: Option<&str>,
    gate_envelope: Option<&Path>,
    call_id: Option<&str>,
    config: &archon_core::config::ArchonConfig,
    env_vars: &archon_core::env_vars::ArchonEnvVars,
) -> Result<()> {
    let mode = config.workflow.gate_mode;
    if mode == archon_core::config::GateMode::Off {
        return Err(anyhow!(
            "gate_mode=off must return before staged task-set lint"
        ));
    }
    if task_file.is_some() || spec_file.is_some() || graph.is_some() {
        return Err(anyhow!(
            "trusted staged task-set lint accepts only --tasks <DIR>"
        ));
    }
    let tasks =
        tasks.ok_or_else(|| anyhow!("trusted staged task-set lint requires --tasks <DIR>"))?;
    let gate_envelope = gate_envelope
        .ok_or_else(|| anyhow!("trusted staged task-set lint requires --gate-envelope <PATH>"))?;
    let call_id = call_id
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| anyhow!("trusted staged task-set lint requires --call-id <ID>"))?;
    let source = crate::command::topology_lint::LintSource::Tasks(tasks.to_path_buf());
    let tasks_root = if tasks.is_absolute() {
        tasks.to_path_buf()
    } else {
        cwd.join(tasks)
    };
    let waivers = crate::command::topology_lint::recorded_waivers(cwd, &tasks_root);
    let factory =
        crate::command::pipeline_workflow_llm::SubagentPipelineClientFactory::configured_only(
            config, env_vars,
        );
    let client = factory
        .build_client(archon_workflow::WorkflowLlmClientRequest {
            cwd: cwd.to_path_buf(),
            origin: "workflow-decompose-task-set-lint".into(),
            session_id: call_id.to_string(),
        })
        .await
        .map_err(anyhow::Error::new);
    let evaluation = match crate::command::topology_lint::evaluate_lint_with_fidelity(
        cwd, &source, mode, client, &waivers,
    )
    .await
    {
        Ok(evaluation) => evaluation,
        Err(error) => crate::command::workflow_gate::GateEvaluation::new("", Vec::new())
            .with_operational_error(error.to_string()),
    };
    let staging_root = gate_envelope
        .parent()
        .ok_or_else(|| anyhow!("staged task-set envelope has no parent"))?;
    let manifest = crate::command::workflow_gate_envelope::stage_gate_evaluation(
        staging_root,
        gate_envelope,
        call_id,
        "task-set-lint",
        evaluation,
        Vec::new(),
    )?;
    println!("{}", serde_json::to_string(&manifest)?);
    Ok(())
}

/// The decomposition's body gate (`land-task-body`).
///
/// The mechanical body checks run first; when they pass, the candidate is
/// judged for obligation fidelity against the obligations it claims
/// (Issue-44). The set gate keeps the same audit, but there a false verdict
/// stops the run: it has no author to send the finding back to. Here the
/// body author still has attempts, so each false verdict lands as a `Body`
/// finding it is asked to repair. The critic client, waivers, and the
/// operational-not-pass rule are exactly the set gate's.
#[allow(clippy::too_many_arguments)]
pub(super) async fn handle_staged_task_file_lint(
    cwd: &Path,
    task_file: Option<&Path>,
    tasks: Option<&Path>,
    spec_file: Option<&Path>,
    graph: Option<&str>,
    candidate_stdin: bool,
    staging_root: Option<&Path>,
    gate_envelope: Option<&Path>,
    call_id: Option<&str>,
    config: &archon_core::config::ArchonConfig,
    env_vars: &archon_core::env_vars::ArchonEnvVars,
) -> Result<()> {
    let mode = config.workflow.gate_mode;
    if mode == archon_core::config::GateMode::Off {
        return Err(anyhow!(
            "gate_mode=off must return before staged task-file lint"
        ));
    }
    if !candidate_stdin {
        return Err(anyhow!(
            "trusted staged task-file lint requires --candidate-stdin"
        ));
    }
    if tasks.is_some() || spec_file.is_some() || graph.is_some() {
        return Err(anyhow!(
            "trusted staged task-file lint accepts only --task-file <PATH>"
        ));
    }
    let task_file = task_file
        .ok_or_else(|| anyhow!("trusted staged task-file lint requires --task-file <PATH>"))?;
    let staging_root = staging_root
        .ok_or_else(|| anyhow!("trusted staged task-file lint requires --staging-root <DIR>"))?;
    let gate_envelope = gate_envelope
        .ok_or_else(|| anyhow!("trusted staged task-file lint requires --gate-envelope <PATH>"))?;
    let call_id = call_id
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| anyhow!("trusted staged task-file lint requires --call-id <ID>"))?;
    let mut candidate = Vec::new();
    std::io::stdin()
        .take(archon_workflow::HostCommandRequest::MAX_STDIN_BYTES as u64 + 1)
        .read_to_end(&mut candidate)?;
    if candidate.len() > archon_workflow::HostCommandRequest::MAX_STDIN_BYTES {
        return Err(anyhow!(
            "candidate stdin exceeds {} bytes",
            archon_workflow::HostCommandRequest::MAX_STDIN_BYTES
        ));
    }
    let path = if task_file.is_absolute() {
        task_file.to_path_buf()
    } else {
        cwd.join(task_file)
    };
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| anyhow!("candidate TASK path has no UTF-8 file name"))?
        .to_string();
    let evaluation =
        crate::command::topology_lint::evaluate_task_file_candidate(cwd, &path, &candidate, mode)?;
    let evaluation = if evaluation.findings.is_empty() && evaluation.operational_error().is_none() {
        audit_candidate_fidelity(
            cwd, &path, &candidate, call_id, config, env_vars, evaluation,
        )
        .await
    } else {
        evaluation
    };
    let manifest = crate::command::workflow_gate_envelope::stage_gate_evaluation(
        staging_root,
        gate_envelope,
        call_id,
        "land-task-body",
        evaluation,
        vec![crate::command::workflow_gate_envelope::StagedGateOutput {
            relative_path: file_name,
            bytes: candidate,
        }],
    )?;
    println!("{}", serde_json::to_string(&manifest)?);
    Ok(())
}

/// Issue-44: the fidelity section of the body gate. Runs only once the body
/// is mechanically clean, so the critic is never spent on a body the author
/// is already being sent back for. A candidate that reached here parsed, so
/// a parse failure inside the audit is operational, like every other failure.
async fn audit_candidate_fidelity(
    cwd: &Path,
    path: &Path,
    candidate: &[u8],
    call_id: &str,
    config: &archon_core::config::ArchonConfig,
    env_vars: &archon_core::env_vars::ArchonEnvVars,
    mut evaluation: crate::command::workflow_gate::GateEvaluation,
) -> crate::command::workflow_gate::GateEvaluation {
    let tasks_root = path.parent().unwrap_or(path);
    let waivers = crate::command::topology_lint::recorded_waivers(cwd, tasks_root);
    let factory =
        crate::command::pipeline_workflow_llm::SubagentPipelineClientFactory::configured_only(
            config, env_vars,
        );
    let client = factory
        .build_client(archon_workflow::WorkflowLlmClientRequest {
            cwd: cwd.to_path_buf(),
            origin: "workflow-decompose-land-task-body".into(),
            session_id: call_id.to_string(),
        })
        .await
        .map_err(anyhow::Error::new);
    let outcome = match std::str::from_utf8(candidate) {
        Ok(raw) => {
            crate::command::topology_lint::audit_task_file_candidate(
                cwd, path, raw, client, &waivers,
            )
            .await
        }
        Err(error) => Err(anyhow::Error::new(error).context("candidate body is not UTF-8")),
    };
    match outcome {
        Ok((report, findings)) => {
            evaluation.report.push_str(&report);
            evaluation.findings.extend(findings);
            evaluation
        }
        Err(error) => evaluation.with_operational_error(format!(
            "obligation fidelity audit failed operationally: {error:#}"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Issue-44: the body gate must refuse, not pass, when the critic cannot
    /// be reached — the same rule as the set gate. A mechanically clean body
    /// with no provider configured leaves the envelope with an operational
    /// error naming the fidelity audit and no invented finding.
    #[tokio::test]
    async fn the_body_gate_reports_an_unreachable_critic_as_operational_never_a_pass() {
        let temp = tempfile::tempdir().expect("tempdir");
        let cwd = temp.path();
        let tasks = cwd.join("tasks").join("PRD-WS-001");
        std::fs::create_dir_all(&tasks).expect("tasks");
        std::fs::write(
            cwd.join("tasks").join("PRD-WS-001.md"),
            "## Acceptance Criteria\n| ID | Criterion |\n|---|---|\n| AC-WS-001 | Widgets land. |\n",
        )
        .expect("prd");
        let candidate = "# TASK-WS-001\n\n```yaml\ntask_id: TASK-WS-001\ntitle: T\ncomplexity: small\nstatus: pending\ndepends_on: []\nblocks: []\nimplements: [\"AC-WS-001\"]\nrequired_env_keys: []\nrequired_tools: [cargo]\ndeliverable_contracts: []\n```\n\n## Focused Tests\n\n- `cargo test -p w`\n";
        let path = tasks.join("TASK-WS-001.md");
        let clean = crate::command::topology_lint::evaluate_task_file_candidate(
            cwd,
            &path,
            candidate.as_bytes(),
            archon_core::config::GateMode::Enforce,
        )
        .expect("mechanical checks");
        assert!(clean.findings.is_empty(), "{}", clean.report);
        let mut config = archon_core::config::ArchonConfig::default();
        config.workflow.gate_mode = archon_core::config::GateMode::Enforce;
        let env = archon_core::env_vars::load_env_vars_from(&std::collections::HashMap::new());
        let evaluation = audit_candidate_fidelity(
            cwd,
            &path,
            candidate.as_bytes(),
            "call-1",
            &config,
            &env,
            clean,
        )
        .await;
        let error = evaluation
            .operational_error()
            .expect("operational error recorded");
        assert!(
            error.contains("obligation fidelity audit failed operationally"),
            "{error}"
        );
        assert!(error.contains("critic client"), "{error}");
        assert!(
            evaluation.findings.is_empty(),
            "no finding stands in for a verdict"
        );
        assert!(!path.exists(), "the gate publishes nothing on its own");
    }

    /// The freeze point must refuse, not pass, when the critic cannot be
    /// reached: an unconfigured provider yields an envelope whose
    /// `operational_error` names the fidelity audit, and no policy finding is
    /// invented to stand in for the verdict that was never given.
    #[tokio::test]
    async fn the_set_gate_reports_an_unreachable_critic_as_operational_never_a_pass() {
        let temp = tempfile::tempdir().expect("tempdir");
        let cwd = temp.path();
        let tasks = cwd.join("tasks").join("PRD-WS-001");
        std::fs::create_dir_all(&tasks).expect("tasks");
        std::fs::write(
            cwd.join("tasks").join("PRD-WS-001.md"),
            "## Acceptance Criteria\n| ID | Criterion |\n|---|---|\n| AC-WS-001 | Widgets land. |\n",
        )
        .expect("prd");
        std::fs::write(
            tasks.join("TASK-WS-001.md"),
            "# TASK-WS-001\n\n```yaml\ntask_id: TASK-WS-001\ntitle: T\ncomplexity: small\nstatus: pending\ndepends_on: []\nblocks: []\nimplements: [\"AC-WS-001\"]\nrequired_env_keys: []\nrequired_tools: [cargo]\ndeliverable_contracts: []\n```\n\n## Focused Tests\n\n- `cargo test -p w`\n",
        )
        .expect("task");
        let staging = cwd.join("staging");
        std::fs::create_dir_all(&staging).expect("staging");
        let envelope = staging.join("gate-envelope.json");
        let mut config = archon_core::config::ArchonConfig::default();
        config.workflow.gate_mode = archon_core::config::GateMode::Enforce;
        let env = archon_core::env_vars::load_env_vars_from(&std::collections::HashMap::new());
        handle_staged_task_set_lint(
            cwd,
            None,
            Some(&tasks),
            None,
            None,
            Some(&envelope),
            Some("call-1"),
            &config,
            &env,
        )
        .await
        .expect("the staged child reports through the envelope, never by exiting non-zero");
        let text = std::fs::read_to_string(&envelope).expect("envelope written");
        let value: serde_json::Value = serde_json::from_str(&text).expect("envelope json");
        let error = value["operational_error"]["text"]
            .as_str()
            .expect("operational error recorded");
        assert!(
            error.contains("obligation fidelity audit failed operationally"),
            "{error}"
        );
        assert!(error.contains("critic client"), "{error}");
    }
}
