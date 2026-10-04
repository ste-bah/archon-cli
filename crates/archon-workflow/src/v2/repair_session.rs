//! A validation repair chain has one identity even when call ids are reused.
//!
//! The identity can be replaced within its scope: when a continuation is
//! refused, the new agent's session becomes the call's session, so every
//! later repair or author attempt continues the new agent (#241).
type Generation = std::sync::Arc<std::sync::Mutex<String>>;
tokio::task_local! { static GENERATION: Generation; }

pub fn current() -> Option<String> {
    GENERATION
        .try_with(|generation| generation.lock().unwrap().clone())
        .ok()
}

/// Make `id` the current identity for the rest of this scope, and for a
/// remembered author session that had the one it replaces.
pub fn replace_current(id: String) {
    let Ok(old) = GENERATION
        .try_with(|generation| std::mem::replace(&mut *generation.lock().unwrap(), id.clone()))
    else {
        return;
    };
    let _ = AUTHOR.try_with(|session| {
        if let Some((remembered, _)) = session.0.lock().unwrap().as_mut()
            && *remembered == old
        {
            *remembered = id;
        }
    });
}

fn generation(id: String) -> Generation {
    std::sync::Arc::new(std::sync::Mutex::new(id))
}

pub async fn scope<T>(work: impl std::future::Future<Output = T>) -> T {
    GENERATION
        .scope(generation(uuid::Uuid::new_v4().to_string()), work)
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
    if request.call.id == "author-workflow-script"
        && let Some(id) = current()
    {
        let _ = AUTHOR.try_with(|session| *session.0.lock().unwrap() = Some((id, request.clone())));
    }
}
pub async fn scope_id<T>(id: String, work: impl std::future::Future<Output = T>) -> T {
    GENERATION.scope(generation(id), work).await
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
