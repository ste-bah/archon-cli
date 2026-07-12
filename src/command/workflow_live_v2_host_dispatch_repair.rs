async fn run_v2_agent_repair_with_rejected_output_log(
    adapter: &WorkflowV2AgentAdapter,
    client: &LiveV2AgentClient,
    request: &archon_workflow::WorkflowV2AgentRequest,
    v2_store: Option<&WorkflowV2ResultStore>,
    first: String,
    first_error: WorkflowV2AgentError,
) -> Result<WorkflowV2Result, WorkflowV2AgentError> {
    let prompt = adapter.build_repair_prompt(request, &first, &first_error);
    let (repaired, first_error) =
        request_repair_output(client, request, prompt, first_error).await?;
    match adapter.parse_agent_output(request, &repaired) {
        Ok(result) => {
            save_rejected_write_result(v2_store, request, "repair", &repaired, &result);
            Ok(result)
        }
        Err(repair_error) if repair_error.differs_from(&first_error) => {
            save_rejected_write_output(v2_store, request, "repair", &repaired, &repair_error);
            run_v2_agent_second_repair(
                adapter, client, request, v2_store, repaired, first_error, repair_error,
            )
            .await
        }
        Err(repair_error) => {
            save_rejected_write_output(v2_store, request, "repair", &repaired, &repair_error);
            Err(repair_exhausted_error(first_error, repair_error))
        }
    }
}

async fn run_v2_agent_second_repair(
    adapter: &WorkflowV2AgentAdapter,
    client: &LiveV2AgentClient,
    request: &archon_workflow::WorkflowV2AgentRequest,
    v2_store: Option<&WorkflowV2ResultStore>,
    repaired: String,
    first_error: WorkflowV2AgentError,
    repair_error: WorkflowV2AgentError,
) -> Result<WorkflowV2Result, WorkflowV2AgentError> {
    let prompt = adapter.build_repair_prompt(request, &repaired, &repair_error);
    let (second, first_error) =
        request_repair_output(client, request, prompt, first_error).await?;
    match adapter.parse_agent_output(request, &second) {
        Ok(result) => {
            save_rejected_write_result(v2_store, request, "second_repair", &second, &result);
            Ok(result)
        }
        Err(last_error) => {
            save_rejected_write_output(v2_store, request, "second_repair", &second, &last_error);
            Err(repair_exhausted_error(first_error, last_error))
        }
    }
}

async fn request_repair_output(
    client: &LiveV2AgentClient,
    request: &archon_workflow::WorkflowV2AgentRequest,
    prompt: String,
    first_error: WorkflowV2AgentError,
) -> Result<(String, WorkflowV2AgentError), WorkflowV2AgentError> {
    match client.run_agent_request(request, prompt).await {
        Ok(output) => Ok((output, first_error)),
        Err(last) => Err(repair_exhausted_error(first_error, last)),
    }
}

fn repair_exhausted_error(
    first: WorkflowV2AgentError,
    last: WorkflowV2AgentError,
) -> WorkflowV2AgentError {
    WorkflowV2AgentError::RepairExhausted {
        first_error: Box::new(first),
        repair_error: Box::new(last),
    }
}
