#[cfg(not(test))]
const TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);
#[cfg(test)]
const TIMEOUT: std::time::Duration = std::time::Duration::from_millis(10);

pub(super) async fn await_shutdown<S, F>(
    server: S,
    shutdown: F,
    shutdown_tx: tokio::sync::oneshot::Sender<()>,
) -> anyhow::Result<()>
where
    S: std::future::IntoFuture<Output = std::io::Result<()>>,
    F: std::future::Future<Output = ()>,
{
    let mut server = Box::pin(server.into_future());
    let mut shutdown = Box::pin(shutdown);
    tokio::select! {
        result = &mut server => result.map_err(|error| anyhow::anyhow!("web: server error: {error}")),
        _ = &mut shutdown => {
            let _ = shutdown_tx.send(());
            tokio::time::timeout(TIMEOUT, server)
                .await
                .map_err(|_| anyhow::anyhow!("web: graceful server shutdown timed out after {TIMEOUT:?}"))?
                .map_err(|error| anyhow::anyhow!("web: server error: {error}"))
        }
    }
}
