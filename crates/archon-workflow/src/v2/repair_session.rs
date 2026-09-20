//! A validation repair chain has one identity even when call ids are reused.
tokio::task_local! { static GENERATION: String; }

pub fn current() -> Option<String> {
    GENERATION.try_with(Clone::clone).ok()
}

pub async fn scope<T>(work: impl std::future::Future<Output = T>) -> T {
    GENERATION
        .scope(uuid::Uuid::new_v4().to_string(), work)
        .await
}

#[derive(Default)]
pub struct AuthorSession(std::sync::Mutex<Option<(String, super::WorkflowV2AgentRequest)>>);
tokio::task_local! { static AUTHOR: std::sync::Arc<AuthorSession>; }
pub async fn author_scope<T>(work: impl std::future::Future<Output = T>) -> T {
    AUTHOR
        .scope(std::sync::Arc::new(AuthorSession::default()), work)
        .await
}
pub fn author_previous(
    request: &super::WorkflowV2AgentRequest,
) -> Option<(String, super::WorkflowV2AgentRequest)> {
    if request.call.id != "author-workflow-script" {
        return None;
    }
    AUTHOR
        .try_with(|session| session.0.lock().unwrap().clone())
        .ok()
        .flatten()
}
pub fn remember_author(request: &super::WorkflowV2AgentRequest) {
    if request.call.id == "author-workflow-script" {
        if let Some(id) = current() {
            let _ =
                AUTHOR.try_with(|session| *session.0.lock().unwrap() = Some((id, request.clone())));
        }
    }
}
pub async fn scope_id<T>(id: String, work: impl std::future::Future<Output = T>) -> T {
    GENERATION.scope(id, work).await
}

pub fn author_current() -> Option<std::sync::Arc<AuthorSession>> {
    AUTHOR.try_with(Clone::clone).ok()
}
pub async fn inherit_author<T>(
    session: Option<std::sync::Arc<AuthorSession>>,
    work: impl std::future::Future<Output = T>,
) -> T {
    match session {
        Some(session) => AUTHOR.scope(session, work).await,
        None => work.await,
    }
}
