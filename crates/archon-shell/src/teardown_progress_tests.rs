use super::*;
const BOUND: Duration = Duration::from_millis(300);
const STEP: Duration = Duration::from_millis(80);

#[test]
fn progressing_membership_can_outlast_the_initial_window() {
    let progress = Progress::new(BOUND);
    let mut active = 5;
    assert_eq!(
        confirm_empty(&progress, || {
            std::thread::sleep(STEP);
            active -= 1;
            Ok(active)
        })
        .unwrap(),
        0
    );
    // A decrease is progress even after a preceding increase. A historical
    // low-water mark must not turn repeated successful exits into a total cap.
    let progress = Progress::new(BOUND);
    let mut counts = [5, 6, 5, 6, 5, 6, 5, 0].into_iter();
    assert_eq!(
        confirm_empty(&progress, || {
            std::thread::sleep(STEP);
            Ok(counts.next().unwrap())
        })
        .unwrap(),
        0
    );
    stable_or_increasing_membership_does_not_extend_the_window();
    failed_accounting_never_confirms_exit();
}

#[test]
fn successive_identity_reads_can_outlast_the_initial_window() {
    let progress = Progress::new(BOUND);
    for _ in 0..5 {
        std::thread::sleep(STEP);
        progress
            .check()
            .expect("successful identity reads are progress");
        progress.advance();
    }
}

#[tokio::test]
async fn outer_watchdog_observes_identity_and_membership_progress() {
    let progress = Progress::new(BOUND);
    let worker_progress = progress.clone();
    let task = tokio::task::spawn_blocking(move || {
        for _ in 0..5 {
            std::thread::sleep(STEP);
            worker_progress.advance();
        }
        0
    });
    assert_eq!(progress.watch(task).await.unwrap(), 0);
    queued_worker_without_progress_stalls().await;
}

fn stable_or_increasing_membership_does_not_extend_the_window() {
    for count in [vec![3, 3], vec![3, 4, 4]] {
        let progress = Progress::new(BOUND);
        let mut index = 0;
        let active = confirm_empty(&progress, || {
            let active = count[index.min(count.len() - 1)];
            index += 1;
            Ok(active)
        })
        .unwrap();
        assert!(active > 0);
    }
}

async fn queued_worker_without_progress_stalls() {
    let progress = Progress::new(BOUND);
    let task = tokio::spawn(std::future::pending::<()>());
    let abort = task.abort_handle();
    let result = progress.watch(task).await;
    abort.abort();
    assert_eq!(result.unwrap_err().kind(), io::ErrorKind::TimedOut);
}

fn failed_accounting_never_confirms_exit() {
    let progress = Progress::new(BOUND);
    assert!(confirm_empty(&progress, || Err(io::Error::other("probe refused"))).is_err());
}
