use super::*;
fn unknown_settlement_retains_pins(phase: u32) {
    let run = tempfile::tempdir().unwrap();
    let id = ended_group();
    let guard = record_group(
        &run.path().join(GROUP_RECORDS_DIR),
        id,
        id,
        None,
        None,
        "cmd",
    )
    .unwrap();
    let evidence = guard.evidence().unwrap();
    let (mut survivor, pin) = sleeper();
    evidence.remember(&[pin]).unwrap();
    match phase {
        0 => {}
        1 => evidence.begin().unwrap(),
        _ => evidence.complete(&[pin]).unwrap(),
    }
    let path = guard.path().to_path_buf();
    guard.keep(None); // additional failure: membership may now be incomplete
    let settled = read(&path);
    let refused_alive = require_no_running_groups(run.path(), "run");
    let _ = survivor.kill();
    let _ = survivor.wait();
    let refused_gone = require_no_running_groups(run.path(), "run");
    assert_eq!(
        settled.survivors,
        vec![pin],
        "unknown settlement erased known pins"
    );
    assert!(settled.survivors_unknown);
    assert!(refused_alive.is_err());
    assert!(
        refused_gone.is_err(),
        "incomplete membership must never falsely heal"
    );
}
#[test]
fn unknown_settlement_retains_scanned_identities() {
    unknown_settlement_retains_pins(0);
}
#[test]
fn unknown_settlement_retains_freeze_identities() {
    unknown_settlement_retains_pins(1);
}
#[test]
fn unknown_settlement_retains_confirmed_survivor_identities() {
    unknown_settlement_retains_pins(2);
}
