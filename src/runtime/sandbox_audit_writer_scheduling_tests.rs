use super::*;

fn session() -> SandboxAuditWrite {
    SandboxAuditWrite::Session(Box::new(
        archon_learning::sandbox_sessions::SandboxSessionRecord::new(
            "gated-failure",
            "local",
            "test-profile",
            "configured",
            chrono::Utc::now().to_rfc3339(),
        ),
    ))
}

#[tokio::test(start_paused = true)]
async fn shutdown_deadline_reports_exact_unresolved_while_worker_is_gated() {
    let db = crate::command::test_support::registered_learning_test_db("audit-gated");
    db.run_script(
        "{::remove sandbox_sessions}",
        Default::default(),
        cozo::ScriptMutability::Mutable,
    )
    .unwrap();
    let (writer, receiver) = SandboxAuditWriter::bounded_for_test(2);
    let (release, start) = oneshot::channel();
    let (finished, completion) = oneshot::channel();
    let counters = Arc::clone(&writer.counters);
    let worker_db = db.arc();
    let worker = tokio::spawn(async move {
        start.await.unwrap();
        run_writer(worker_db, receiver, counters).await;
        finished.send(()).unwrap();
    });
    writer.enqueue(session());
    let drain = SandboxAuditDrain {
        writer: writer.clone(),
        worker,
    };
    let shutdown = drain.shutdown(Duration::from_secs(2));
    tokio::pin!(shutdown);
    // Poll shutdown to its first pending point: its deadline and Shutdown
    // message are installed, while persistence is explicitly held closed.
    tokio::select! {
        biased;
        result = &mut shutdown => panic!("gated shutdown completed: {result:?}"),
        _ = std::future::ready(()) => {}
    }
    tokio::time::advance(Duration::from_secs(2)).await;
    let error = shutdown.await.unwrap_err();
    let message = error.to_string();
    for expected in [
        "timed out",
        "accepted=1",
        "persisted=0",
        "failed=0",
        "dropped=0",
        "unresolved=1",
    ] {
        assert!(message.contains(expected), "{error:#}");
    }
    // The timeout detaches the worker. Keep its private DB alive until it has
    // really consumed Shutdown, then verify the eventual terminal counts.
    release.send(()).unwrap();
    completion.await.unwrap();
    assert_eq!(
        writer.readback_for_test(),
        SandboxAuditReadback {
            accepted: 1,
            persisted: 0,
            failed: 1,
            dropped: 0
        }
    );
}

#[tokio::test(start_paused = true)]
async fn shutdown_ack_follows_all_terminal_counters_after_delayed_worker_start() {
    let db = crate::command::test_support::registered_learning_test_db("audit-delayed");
    db.run_script(
        "{::remove sandbox_sessions}",
        Default::default(),
        cozo::ScriptMutability::Mutable,
    )
    .unwrap();
    let (writer, receiver) = SandboxAuditWriter::bounded_for_test(2);
    let (release, start) = oneshot::channel();
    let counters = Arc::clone(&writer.counters);
    let worker_db = db.arc();
    let worker = tokio::spawn(async move {
        start.await.unwrap();
        run_writer(worker_db, receiver, counters).await;
    });
    writer.enqueue(session());
    let drain = SandboxAuditDrain { writer, worker };
    let shutdown = drain.shutdown(Duration::from_secs(2));
    tokio::pin!(shutdown);
    tokio::select! {
        biased;
        result = &mut shutdown => panic!("gated shutdown completed: {result:?}"),
        _ = std::future::ready(()) => {}
    }
    release.send(()).unwrap();
    let error = shutdown.await.unwrap_err();
    let message = error.to_string();
    for expected in [
        "evidence incomplete",
        "accepted=1",
        "persisted=0",
        "failed=1",
        "dropped=0",
        "unresolved=0",
    ] {
        assert!(message.contains(expected), "{error:#}");
    }
}
