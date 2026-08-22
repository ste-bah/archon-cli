//! Cancellation safety for the `WebSearch` call budget (#200 Phase 1).

use std::time::Duration;

use tokio::io::AsyncReadExt;

use super::{CALL_BUDGET, TIMEOUT, WebSearchTool, fetch_search_html};
use crate::execution_deadline::ExecutionDeadline;
use crate::tool::Tool;

#[test]
fn declares_a_call_budget_above_the_reqwest_timeout() {
    let budget = WebSearchTool
        .timeout()
        .expect("WebSearch declares a budget");
    assert_eq!(budget, CALL_BUDGET);
    assert!(
        budget > TIMEOUT,
        "the dispatcher budget is a backstop, not the primary bound"
    );
}

/// Opting a tool in means its future gets dropped mid-await at the deadline.
/// For `WebSearch` the resource at stake is the connection to the search
/// endpoint, so this stalls a real server, cancels the call, and requires the
/// server to observe the connection close rather than being left hanging.
#[tokio::test]
async fn dropping_the_search_at_the_deadline_closes_the_connection() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind loopback");
    let addr = listener.local_addr().expect("local addr");

    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.expect("client connects");
        let mut buf = [0u8; 4096];
        let request_bytes = socket.read(&mut buf).await.expect("request arrives");
        assert!(request_bytes > 0, "expected an HTTP request");
        // Never answer: the next read only returns once the client closes.
        tokio::time::timeout(Duration::from_secs(10), socket.read(&mut buf)).await
    });

    let endpoint = format!("http://{addr}/html/");
    let outcome = ExecutionDeadline::new(Duration::from_millis(300))
        .wait(fetch_search_html(&endpoint, "archon"))
        .await;

    assert!(outcome.is_none(), "the stalled search must not complete");
    let after_drop = server
        .await
        .expect("server task")
        .expect("server must see the connection close, not hang for 10s");
    assert_eq!(
        after_drop.expect("read after drop"),
        0,
        "dropping the search must close the client half of the connection"
    );
}
