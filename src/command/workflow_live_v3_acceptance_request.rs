use archon_workflow::v2::acceptance_stage::{ACCEPTANCE_MAX_ROUNDS, ACCEPTANCE_STAGE_TOOL};
use archon_workflow::{WorkflowError, WorkflowResult, WorkflowV2CallExecution};

pub(super) struct StageRequest {
    pub(super) round: u32,
    pub(super) max_rounds: u32,
    pub(super) check_ids: Vec<String>,
}

pub(super) fn parse_request(execution: &WorkflowV2CallExecution) -> WorkflowResult<StageRequest> {
    let extra = &execution.call.options.extra;
    let round = extra
        .get("round")
        .and_then(serde_json::Value::as_u64)
        .filter(|round| *round >= 1)
        .ok_or_else(|| {
            WorkflowError::SpecInvalid(format!(
                "{ACCEPTANCE_STAGE_TOOL} requires a positive `round`; the prelude's acceptance() primitive supplies it"
            ))
        })?;
    // Recorded only: the loop's budget follows progress (A2), never a count.
    let max_rounds = extra
        .get("maxRounds")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(u64::from(ACCEPTANCE_MAX_ROUNDS))
        .clamp(1, u64::from(u32::MAX));
    let check_ids = extra
        .get("checkIds")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(str::to_string)
        .collect();
    Ok(StageRequest {
        round: u32::try_from(round).unwrap_or(u32::MAX),
        max_rounds: max_rounds as u32,
        check_ids,
    })
}

pub(super) async fn reserve(
    writer: &archon_workflow::stage_write::StageWriter,
    run_dir: &std::path::Path,
    round: u32,
) -> WorkflowResult<archon_workflow::v2::acceptance_stage::RoundReservation> {
    let reserved = archon_workflow::stage_write::scope(writer.clone(), async {
        archon_workflow::v2::acceptance_stage::reserve_round(run_dir, round)
    })
    .await;
    match reserved {
        Ok(reservation) => Ok(reservation),
        Err(error @ (WorkflowError::ControlCancelled(_) | WorkflowError::ControlPaused(_))) => {
            Err(error)
        }
        Err(error) => Err(super::ledger::pause_reservation(writer, round, &error)),
    }
}
