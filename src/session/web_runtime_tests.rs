use std::sync::Arc;

use archon_tui::AgentDispatcher;

use super::{WebSessionHandle, finish_reply, sanitize_web_reply};

#[tokio::test]
async fn shutdown_releases_submission_blocked_on_full_input_queue() {
    let (input_tx, _input_rx) = tokio::sync::mpsc::channel(1);
    input_tx.send("queued".to_string()).await.unwrap();
    let (permission_tx, _permission_rx) = tokio::sync::mpsc::channel(1);
    let (ask_user_tx, _ask_user_rx) = tokio::sync::mpsc::channel(1);
    let (_event_tx, event_rx) = archon_tui::event_channel::bounded_tui_event_channel();
    let session = WebSessionHandle {
        input_tx: tokio::sync::Mutex::new(Some(input_tx)),
        permission_tx,
        ask_user_tx,
        event_rx: tokio::sync::Mutex::new(event_rx),
        last_assistant_response: Arc::new(tokio::sync::Mutex::new(String::new())),
        cancel_handle: Arc::new(std::sync::Mutex::new(None)),
        loop_task: tokio::sync::Mutex::new(None),
        sandbox_audit_drain:
            crate::runtime::sandbox_audit_writer::SandboxAuditDrainHandle::empty_for_test(),
        dispatcher: Arc::new(std::sync::Mutex::new(AgentDispatcher::new(
            Arc::new(crate::agent_handle::NoopAgentRouter),
            tokio::sync::mpsc::unbounded_channel().0,
        ))),
        shutdown: tokio_util::sync::CancellationToken::new(),
    };

    let submit = session.submit("blocked".to_string());
    let shutdown = async {
        tokio::task::yield_now().await;
        session.begin_shutdown().await;
    };
    let (submit_result, ()) = tokio::time::timeout(std::time::Duration::from_secs(1), async {
        tokio::join!(submit, shutdown)
    })
    .await
    .expect("shutdown must release blocked web submission");

    assert!(submit_result.unwrap_err().to_string().contains("shut down"));
}

#[tokio::test]
async fn active_submit_exits_when_shutdown_closes_event_channel() {
    let (input_tx, mut input_rx) = tokio::sync::mpsc::channel(1);
    let (permission_tx, _permission_rx) = tokio::sync::mpsc::channel(1);
    let (ask_user_tx, _ask_user_rx) = tokio::sync::mpsc::channel(1);
    let (event_tx, event_rx) = archon_tui::event_channel::bounded_tui_event_channel();
    let loop_task = tokio::spawn(async move {
        input_rx.recv().await;
        drop(event_tx);
        Ok(())
    });
    let session = WebSessionHandle {
        input_tx: tokio::sync::Mutex::new(Some(input_tx)),
        permission_tx,
        ask_user_tx,
        event_rx: tokio::sync::Mutex::new(event_rx),
        last_assistant_response: Arc::new(tokio::sync::Mutex::new(String::new())),
        cancel_handle: Arc::new(std::sync::Mutex::new(None)),
        loop_task: tokio::sync::Mutex::new(Some(loop_task)),
        sandbox_audit_drain:
            crate::runtime::sandbox_audit_writer::SandboxAuditDrainHandle::empty_for_test(),
        dispatcher: Arc::new(std::sync::Mutex::new(AgentDispatcher::new(
            Arc::new(crate::agent_handle::NoopAgentRouter),
            tokio::sync::mpsc::unbounded_channel().0,
        ))),
        shutdown: tokio_util::sync::CancellationToken::new(),
    };

    let submit = session.submit("active request".to_string());
    let shutdown = async {
        tokio::task::yield_now().await;
        session.begin_shutdown().await;
        session.finish_shutdown().await
    };
    let (submit_result, shutdown_result) =
        tokio::time::timeout(std::time::Duration::from_secs(1), async {
            tokio::join!(submit, shutdown)
        })
        .await
        .expect("active web shutdown must not hang");

    assert!(
        submit_result
            .unwrap_err()
            .to_string()
            .contains("event channel closed")
    );
    shutdown_result.unwrap();
    assert!(session.input_tx.lock().await.is_none());
}

#[tokio::test]
async fn finish_shutdown_times_out_and_aborts_stalled_session_loop() {
    let (input_tx, _input_rx) = tokio::sync::mpsc::channel(1);
    let (permission_tx, _permission_rx) = tokio::sync::mpsc::channel(1);
    let (ask_user_tx, _ask_user_rx) = tokio::sync::mpsc::channel(1);
    let (_event_tx, event_rx) = archon_tui::event_channel::bounded_tui_event_channel();
    let loop_task = tokio::spawn(std::future::pending::<anyhow::Result<()>>());
    let session = WebSessionHandle {
        input_tx: tokio::sync::Mutex::new(Some(input_tx)),
        permission_tx,
        ask_user_tx,
        event_rx: tokio::sync::Mutex::new(event_rx),
        last_assistant_response: Arc::new(tokio::sync::Mutex::new(String::new())),
        cancel_handle: Arc::new(std::sync::Mutex::new(None)),
        loop_task: tokio::sync::Mutex::new(Some(loop_task)),
        sandbox_audit_drain:
            crate::runtime::sandbox_audit_writer::SandboxAuditDrainHandle::empty_for_test(),
        dispatcher: Arc::new(std::sync::Mutex::new(AgentDispatcher::new(
            Arc::new(crate::agent_handle::NoopAgentRouter),
            tokio::sync::mpsc::unbounded_channel().0,
        ))),
        shutdown: tokio_util::sync::CancellationToken::new(),
    };

    let error = tokio::time::timeout(std::time::Duration::from_secs(1), session.finish_shutdown())
        .await
        .expect("stalled web session shutdown must resolve before outer deadline")
        .expect_err("stalled web session loop must fail loud after timeout");

    assert!(error.to_string().contains("timed out"), "{error:#}");
    assert!(session.loop_task.lock().await.is_none());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn finish_shutdown_bounds_abort_resistant_session_loop() {
    let (input_tx, _input_rx) = tokio::sync::mpsc::channel(1);
    let (permission_tx, _permission_rx) = tokio::sync::mpsc::channel(1);
    let (ask_user_tx, _ask_user_rx) = tokio::sync::mpsc::channel(1);
    let (_event_tx, event_rx) = archon_tui::event_channel::bounded_tui_event_channel();
    let loop_task = tokio::task::spawn_blocking(|| {
        std::thread::sleep(std::time::Duration::from_millis(200));
    });
    let session = WebSessionHandle {
        input_tx: tokio::sync::Mutex::new(Some(input_tx)),
        permission_tx,
        ask_user_tx,
        event_rx: tokio::sync::Mutex::new(event_rx),
        last_assistant_response: Arc::new(tokio::sync::Mutex::new(String::new())),
        cancel_handle: Arc::new(std::sync::Mutex::new(None)),
        loop_task: tokio::sync::Mutex::new(Some(tokio::spawn(async move {
            loop_task.await.map_err(anyhow::Error::from)?;
            Ok(())
        }))),
        sandbox_audit_drain:
            crate::runtime::sandbox_audit_writer::SandboxAuditDrainHandle::empty_for_test(),
        dispatcher: Arc::new(std::sync::Mutex::new(AgentDispatcher::new(
            Arc::new(crate::agent_handle::NoopAgentRouter),
            tokio::sync::mpsc::unbounded_channel().0,
        ))),
        shutdown: tokio_util::sync::CancellationToken::new(),
    };

    let error = tokio::time::timeout(std::time::Duration::from_secs(1), session.finish_shutdown())
        .await
        .expect("abort-resistant web session shutdown must resolve before outer deadline")
        .expect_err("abort-resistant web session loop must fail loud");

    assert!(error.to_string().contains("timed out"), "{error:#}");
    assert!(session.loop_task.lock().await.is_none());
}

#[tokio::test]
async fn begin_shutdown_closes_web_session_input() {
    let (input_tx, _input_rx) = tokio::sync::mpsc::channel(1);
    let (permission_tx, _permission_rx) = tokio::sync::mpsc::channel(1);
    let (ask_user_tx, _ask_user_rx) = tokio::sync::mpsc::channel(1);
    let (_event_tx, event_rx) = archon_tui::event_channel::bounded_tui_event_channel();
    let loop_task = tokio::spawn(std::future::pending::<anyhow::Result<()>>());
    let session = WebSessionHandle {
        input_tx: tokio::sync::Mutex::new(Some(input_tx)),
        permission_tx,
        ask_user_tx,
        event_rx: tokio::sync::Mutex::new(event_rx),
        last_assistant_response: Arc::new(tokio::sync::Mutex::new(String::new())),
        cancel_handle: Arc::new(std::sync::Mutex::new(None)),
        loop_task: tokio::sync::Mutex::new(Some(loop_task)),
        sandbox_audit_drain:
            crate::runtime::sandbox_audit_writer::SandboxAuditDrainHandle::empty_for_test(),
        dispatcher: Arc::new(std::sync::Mutex::new(AgentDispatcher::new(
            Arc::new(crate::agent_handle::NoopAgentRouter),
            tokio::sync::mpsc::unbounded_channel().0,
        ))),
        shutdown: tokio_util::sync::CancellationToken::new(),
    };

    session.begin_shutdown().await;

    assert!(session.input_tx.lock().await.is_none());
    session.loop_task.lock().await.take().unwrap().abort();
}

#[test]
fn finish_reply_prefers_streamed_text() {
    assert_eq!(finish_reply(" live reply ", "stale"), "live reply");
}

#[test]
fn finish_reply_uses_last_assistant_response_when_stream_empty() {
    assert_eq!(finish_reply("   ", " buffered reply "), "buffered reply");
}

#[test]
fn finish_reply_removes_legacy_tool_transcript_noise() {
    let reply = sanitize_web_reply(
        "\n[tool] DocSearch started\n\
         [tool] memory_recall done: 10 memories found\n\
         noisy memory row\n\
         \n\
         [tool] DocSearch failed: Error: database is locked\n\
         The document store is locked right now.\n",
    );
    assert_eq!(reply, "The document store is locked right now.");
}
