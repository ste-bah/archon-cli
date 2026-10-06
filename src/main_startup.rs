//! Ordering boundary between mode dispatch and interactive resources.

use std::future::Future;

pub(crate) async fn run<S, V, E, VF, IF>(
    dispatch: impl Future<Output = Result<Option<S>, E>>,
    setup_voice: impl FnOnce() -> VF,
    interactive: impl FnOnce(S, V) -> IF,
) -> Result<(), E>
where
    VF: Future<Output = V>,
    IF: Future<Output = Result<(), E>>,
{
    let Some(session) = dispatch.await? else {
        return Ok(());
    };
    let voice = setup_voice().await;
    interactive(session, voice).await
}
