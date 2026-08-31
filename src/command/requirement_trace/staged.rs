//! Typed envelope-only child mode for fixed-decomposition requirement trace.

use std::path::Path;

use anyhow::Result;

use super::{TraceOptions, evaluate_trace_for_published_bodies};

pub(super) fn handle(
    cwd: &Path,
    options: &TraceOptions,
    gate_envelope: Option<&Path>,
    call_id: Option<&str>,
    mode: archon_core::config::GateMode,
) -> Result<()> {
    if mode == archon_core::config::GateMode::Off {
        anyhow::bail!("gate_mode=off must return before staged requirements trace");
    }
    if options.graph.is_some()
        || options.evidence.is_some()
        || options.leann_db.is_some()
        || options.persist.is_some()
        || options.falsify
        || options.json
    {
        anyhow::bail!(
            "trusted staged requirements trace accepts only --prd <PATH> and --tasks <DIR>"
        );
    }
    let gate_envelope = gate_envelope.ok_or_else(|| {
        anyhow::anyhow!("trusted staged requirements trace requires --gate-envelope <PATH>")
    })?;
    let call_id = call_id
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| {
            anyhow::anyhow!("trusted staged requirements trace requires --call-id <ID>")
        })?;
    let evaluation = match evaluate_trace_for_published_bodies(cwd, options) {
        Ok(evaluation) => evaluation,
        Err(error) => crate::command::workflow_gate::GateEvaluation::new("", Vec::new())
            .with_operational_error(error.to_string()),
    };
    let staging_root = gate_envelope
        .parent()
        .ok_or_else(|| anyhow::anyhow!("staged requirements envelope has no parent"))?;
    let manifest = crate::command::workflow_gate_envelope::stage_gate_evaluation(
        cwd,
        staging_root,
        gate_envelope,
        call_id,
        "requirements-trace",
        evaluation,
        Vec::new(),
    )?;
    println!("{}", serde_json::to_string(&manifest)?);
    Ok(())
}
