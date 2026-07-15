use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use tokio::sync::oneshot;

use super::blocking_test_seam::{install_blocking_panic, install_docs_db_wait_hook};
use super::*;

#[tokio::test(flavor = "current_thread")]
#[serial_test::serial]
async fn search_yields_to_other_tokio_tasks_while_database_work_is_held() {
    let control = install_docs_db_wait_hook("docs search");
    let temp = tempfile::tempdir().unwrap();
    let ctx = ToolContext {
        working_dir: temp.path().to_path_buf(),
        ..ToolContext::default()
    };
    let search = tokio::spawn(async move {
        run_search(vec!["docs".into(), "search".into(), "needle".into()], &ctx).await
    });
    let observer = start_progress_observer(control);

    tokio::task::yield_now().await;
    let result = search.await.unwrap();

    assert!(
        observer.join().unwrap(),
        "the Tokio task did not make progress"
    );
    assert!(!result.is_error, "{}", result.content);
}

#[tokio::test(flavor = "current_thread")]
#[serial_test::serial]
async fn answer_yields_to_other_tokio_tasks_while_database_work_is_held() {
    let control = install_docs_db_wait_hook("docs answer");
    let temp = tempfile::tempdir().unwrap();
    let ctx = ToolContext {
        working_dir: temp.path().to_path_buf(),
        ..ToolContext::default()
    };
    let answer = tokio::spawn(async move {
        run_answer(vec!["docs".into(), "answer".into(), "needle".into()], &ctx).await
    });
    let observer = start_progress_observer(control);

    tokio::task::yield_now().await;
    let result = answer.await.unwrap();

    assert!(
        observer.join().unwrap(),
        "the Tokio task did not make progress"
    );
    assert!(!result.is_error, "{}", result.content);
}

#[tokio::test]
async fn list_uses_real_document_database() {
    let temp = tempfile::tempdir().unwrap();
    let ctx = ToolContext {
        working_dir: temp.path().to_path_buf(),
        ..ToolContext::default()
    };

    let result = run_list(25, &ctx).await;

    assert!(!result.is_error, "{}", result.content);
    assert_eq!(result.content, "No documents ingested.");
}

#[tokio::test]
async fn get_reports_missing_document_from_real_document_database() {
    let temp = tempfile::tempdir().unwrap();
    let ctx = ToolContext {
        working_dir: temp.path().to_path_buf(),
        ..ToolContext::default()
    };

    let result = run_get("missing-document".into(), &ctx).await;

    assert!(result.is_error);
    assert_eq!(
        result.content,
        "Error: document not found: missing-document"
    );
}

#[tokio::test]
#[serial_test::serial]
async fn search_surfaces_blocking_task_join_failures() {
    install_blocking_panic("docs search");
    let temp = tempfile::tempdir().unwrap();
    let ctx = ToolContext {
        working_dir: temp.path().to_path_buf(),
        ..ToolContext::default()
    };

    let result = run_search(vec!["docs".into(), "search".into(), "needle".into()], &ctx).await;

    assert!(result.is_error);
    assert!(result.content.contains("docs search blocking task failed"));
}

fn start_progress_observer(control: WaitHookControl) -> thread::JoinHandle<bool> {
    let (start_progress_tx, start_progress_rx) = oneshot::channel();
    let (progress_tx, progress_rx) = mpsc::channel();
    tokio::spawn(async move {
        if start_progress_rx.await.is_ok() {
            let _ = progress_tx.send(());
        }
    });
    thread::spawn(move || control.release_after_progress_check(start_progress_tx, progress_rx))
}

pub(super) struct WaitHookControl {
    pub(super) entered: mpsc::Receiver<()>,
    pub(super) release: mpsc::Sender<()>,
}

impl WaitHookControl {
    fn release_after_progress_check(
        self,
        start_progress: oneshot::Sender<()>,
        progress: mpsc::Receiver<()>,
    ) -> bool {
        let entered = self.entered.recv_timeout(Duration::from_secs(1)).is_ok();
        if entered {
            let _ = start_progress.send(());
        }
        let made_progress = entered && progress.recv_timeout(Duration::from_secs(1)).is_ok();
        let _ = self.release.send(());
        entered && made_progress
    }
}
