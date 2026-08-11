//! The negative control for `preserve_cozodb_memory_persist_gate`:
//! proof that the gate next door can go red.
//!
//! This is spec Validation Criterion 4 — "deleting the CozoDB file
//! between the two instantiations causes failure with 'memory not
//! retrievable after restart'" — run by CI on every pass, instead of
//! asserted by a comment recalling that a person once did it by hand.
//! A record of somebody's recollection decays the moment anything
//! under it changes.
//!
//! It matters more now that criterion 5 is a measurement rather than
//! an assertion: the gate's entire remaining ability to go red rests
//! on the persistence checks, and a gate nobody has watched go red is
//! indistinguishable from a gate that cannot.
//!
//! ## Why it shares the gate's code
//!
//! Same temp store, same write path, same checks — the single
//! difference is that the database is deleted between the two
//! instantiations. `super::checks::check_after_restart` is the one the
//! gate calls too. A control carrying its own copy of the assertions
//! would prove only that the copy works, and would drift from the gate
//! the first time either changed.
//!
//! ## Why it demands `Absent` and not merely a failure
//!
//! [`RestartFinding::Absent`] is returned only when the store reopened
//! *healthily* and specifically reported no such memory. A control
//! that accepted "an error occurred" would also be satisfied by a
//! missing directory, a permissions fault or a poisoned lock, none of
//! which say anything about persistence — it would go green while the
//! gate had quietly lost the ability to see the store at all. Those
//! come back as [`RestartFinding::Broken`], which fails this test as
//! loudly as it fails the gate. A control has to fail for the right
//! reason or it is not a control.

use super::checks::{self, RestartFinding};
use super::{fail_msg, store_and_close, temp_db};

#[test]
fn deleting_the_cozodb_file_between_instantiations_loses_the_memory() {
    let (_tmp, db_path) = temp_db();

    let stored_id = store_and_close(&db_path);
    assert!(
        db_path.exists(),
        "{}",
        fail_msg("control precondition: expected a CozoDB file to delete, found none")
    );

    // The tamper. `store_and_close` has dropped the first instance, so
    // the sqlite handle is closed and the file is removable — on
    // Windows an open handle would refuse this, which is itself a check
    // that the drop released what it claims to.
    std::fs::remove_file(&db_path).unwrap_or_else(|e| {
        panic!(
            "{}",
            fail_msg(&format!(
                "control could not delete the CozoDB file at {}: {e}",
                db_path.display()
            ))
        )
    });

    match checks::check_after_restart(&db_path, &stored_id) {
        RestartFinding::Absent(_) => {}
        RestartFinding::Persisted => panic!(
            "{}",
            fail_msg(
                "NEGATIVE CONTROL FAILED: the memory was still retrievable after its \
                 CozoDB file was deleted. The persistence checks cannot go red, so the \
                 gate proves nothing — fix the checks, not this control"
            )
        ),
        RestartFinding::Broken(detail) => panic!(
            "{}",
            fail_msg(&format!(
                "NEGATIVE CONTROL FAILED for the wrong reason: the store did not come \
                 back healthy-and-empty, so this run says nothing about whether the \
                 persistence checks work — {detail}"
            ))
        ),
    }
}
