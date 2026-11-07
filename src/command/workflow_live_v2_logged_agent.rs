//! Continued structured calls and rejected-output logging.
use super::*;

pub(super) async fn run_logged_agent_inner(
    adapter: &WorkflowV2AgentAdapter,
    client: &LiveV2AgentClient,
    request: &archon_workflow::WorkflowV2AgentRequest,
    v2_store: Option<&WorkflowV2ResultStore>,
) -> Result<WorkflowV2Result, WorkflowV2AgentError> {
    let previous = archon_workflow::v2::repair_session::author_previous(request);
    let generation = previous
        .as_ref()
        .map(|(id, _)| id.clone())
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let prompt = adapter.build_prompt_parts(request).invocation;
    let continuing = previous.is_some();
    let request = previous
        .as_ref()
        .map(|(_, original)| original)
        .unwrap_or(request);
    archon_workflow::v2::repair_session::scope_id(generation, async {
        let first = if continuing {
            archon_workflow::v2::continuation::continue_or_restart(client, request, prompt).await?
        } else {
            client.run_agent_request(request, prompt).await?
        };
        archon_workflow::v2::repair_session::remember_author(request);
        match adapter.parse_agent_output(request, &first) {
            Ok(result) => {
                save_rejected_write_result(v2_store, request, "first", &first, &result);
                Ok(result)
            }
            Err(first_error) => {
                save_rejected_output(v2_store, request, "first", &first, &first_error);
                on_heap(|| {
                    run_v2_agent_repair_with_rejected_output_log(
                        adapter,
                        client,
                        request,
                        v2_store,
                        first,
                        first_error,
                    )
                })
                .await
            }
        }
    })
    .await
}
