use super::*;

pub(super) async fn await_cancellable_inference<F, T>(
    inference: F,
    cancel: Option<&tokio_util::sync::CancellationToken>,
) -> anyhow::Result<T>
where
    F: std::future::Future<Output = anyhow::Result<T>>,
{
    let Some(cancel) = cancel else {
        return inference.await;
    };
    tokio::select! {
        biased;
        _ = cancel.cancelled() => anyhow::bail!("Subagent cancelled during LLM inference"),
        result = inference => result,
    }
}

/// How a tool round ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RoundEnd {
    Finished,
    /// The session's no-progress window ran out.
    TimedOut,
    /// The session was cancelled from above while the round ran.
    Cancelled,
}

/// Grace a cut round's tools get to wind down before they are dropped.
const ROUND_CLEANUP: Duration = Duration::from_secs(2);

pub(super) async fn await_tool_round<F>(
    future: F,
    round_cancel: tokio_util::sync::CancellationToken,
    session_id: &str,
    deadline: Option<tokio::time::Instant>,
    credit: &CargoCredit,
) -> (RoundEnd, Vec<progress_novelty::ObservedCall>)
where
    F: std::future::Future<Output = Vec<progress_novelty::ObservedCall>>,
{
    tokio::pin!(future);
    loop {
        let adjusted = deadline.map(|deadline| deadline + credit.live(session_id));
        let expiry = async {
            match adjusted {
                Some(at) => tokio::time::sleep_until(at).await,
                None => std::future::pending::<()>().await,
            }
        };
        tokio::select! {
            biased;
            _ = expiry => {
                let Some(deadline) = deadline else { continue };
                if tokio::time::Instant::now() < deadline + credit.live(session_id) {
                    continue;
                }
                round_cancel.cancel();
                let _ = tokio::time::timeout(ROUND_CLEANUP, &mut future).await;
                return (RoundEnd::TimedOut, Vec::new());
            }
            result = &mut future => return (RoundEnd::Finished, result),
            // Issue-136: the round's token is the session's child, so a
            // cancelled session is seen here; tools that watch the token end
            // themselves, and the rest are dropped after the grace.
            _ = round_cancel.cancelled() => {
                let _ = tokio::time::timeout(ROUND_CLEANUP, &mut future).await;
                return (RoundEnd::Cancelled, Vec::new());
            }
        }
    }
}
