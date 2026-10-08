//! Windows integration coverage: return only after job accounting is empty.
use super::*;
use std::time::Instant;

fn fixture(rest: &str) -> confine::OwnedCheck {
    let mut command = archon_shell::spawn::tokio_command("cmd");
    command
        .args([
            "/C",
            &format!("start /B ping -n 30 127.0.0.1 >NUL & {rest}"),
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    spawn_confined(command, &std::env::temp_dir()).unwrap()
}
fn wait_for_members(owned: &confine::OwnedCheck, count: u32) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while owned.confinement.job.active_processes().unwrap() < count {
        assert!(
            Instant::now() < deadline,
            "fixture descendants never started"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}
#[tokio::test]
async fn successful_leader_exit_still_confirms_remaining_descendants() {
    let mut owned = fixture("exit 0");
    tokio::time::timeout(Duration::from_secs(5), owned.child.wait())
        .await
        .unwrap()
        .unwrap();
    wait_for_members(&owned, 1);
    owned.confinement.kill().await.unwrap();
    assert_eq!(owned.confinement.job.active_processes().unwrap(), 0);
}
#[tokio::test]
async fn interruption_confirms_the_job_before_reaping_the_leader() {
    let mut owned = fixture("ping -n 30 127.0.0.1 >NUL");
    wait_for_members(&owned, 2);
    let mut stall = None;
    let status = terminate(&mut owned.child, &owned.confinement, &mut stall).await;
    assert!(status.is_some());
    assert!(stall.is_none(), "{stall:?}");
    assert_eq!(owned.confinement.job.active_processes().unwrap(), 0);
}
#[tokio::test]
async fn dropping_the_runner_ends_every_recorded_job_member() {
    let owned = fixture("ping -n 30 127.0.0.1 >NUL");
    wait_for_members(&owned, 2);
    let pins = owned
        .confinement
        .job
        .process_identities(Duration::from_secs(3))
        .unwrap();
    drop(owned);
    let mut deadline = Instant::now() + Duration::from_secs(8);
    let mut previous = pins.len();
    loop {
        let live = pins
            .iter()
            .filter(|(pid, start)| {
                archon_shell::job_object::identity_of(*pid).unwrap() == Some(*start)
            })
            .count();
        if live == 0 {
            break;
        }
        if live < previous {
            deadline = Instant::now() + Duration::from_secs(8);
        }
        previous = live;
        assert!(
            Instant::now() < deadline,
            "a job member outlived the dropped runner without progress"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}
