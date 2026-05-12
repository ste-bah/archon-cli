use std::sync::Arc;

use archon_llm::provider::{LlmProvider, LlmRequest};
use archon_llm::streaming::StreamEvent;
use archon_tui::app::TuiEvent;
use archon_tui::event_channel::TuiEventSender;
use archon_tui::observability;

pub(super) fn spawn_btw_loop(
    mut rx: tokio::sync::mpsc::Receiver<String>,
    tui_tx: TuiEventSender,
    provider: Arc<dyn LlmProvider>,
    model: String,
    max_tokens: u32,
    system_prompt: Vec<serde_json::Value>,
) {
    observability::spawn_named("btw-request-loop", async move {
        while let Some(question) = rx.recv().await {
            let tui_tx = tui_tx.clone();
            let provider = Arc::clone(&provider);
            let model = model.clone();
            let sys_prompt = system_prompt.clone();
            observability::spawn_named("btw-response", async move {
                let response =
                    answer_btw_question(provider, model, max_tokens, sys_prompt, question).await;
                let _ = tui_tx.send(TuiEvent::BtwResponse(response));
            });
        }
    });
}

async fn answer_btw_question(
    provider: Arc<dyn LlmProvider>,
    model: String,
    max_tokens: u32,
    system_prompt: Vec<serde_json::Value>,
    question: String,
) -> String {
    let wrapped = format!(
        "<system-reminder>This is a side question from the user. Answer directly in a single response.\n\
         You have NO tools available. This is a one-off response.\n\
         Do NOT say \"Let me check\" or promise actions.</system-reminder>\n\n{question}"
    );
    let request = LlmRequest {
        model,
        max_tokens,
        system: system_prompt,
        messages: vec![serde_json::json!({
            "role": "user",
            "content": [{"type": "text", "text": wrapped}],
        })],
        tools: Vec::new(),
        request_origin: Some("btw".into()),
        ..LlmRequest::default()
    };
    match provider.stream(request).await {
        Ok(mut rx) => {
            let mut response = String::new();
            while let Some(event) = rx.recv().await {
                if let StreamEvent::TextDelta { ref text, .. } = event {
                    response.push_str(text);
                }
            }
            response
        }
        Err(e) => format!("Error: {e}"),
    }
}
