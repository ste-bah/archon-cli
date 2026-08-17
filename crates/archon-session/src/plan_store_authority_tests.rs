use super::*;

#[test]
fn authority_rejects_a_different_secret_after_initialization() {
    let db = test_db();
    let store = PlanStore::new(&db).expect("init");
    let session_id = "authority-wrong-secret";
    let authority = store
        .bootstrap_approval_authority(session_id, [1; 32])
        .expect("initialize authority");

    store
        .validate_approval_authority(&authority, session_id)
        .expect("original authority remains valid");
    let different_secret = match store.bootstrap_approval_authority(session_id, [2; 32]) {
        Ok(_) => panic!("a different secret must not replace the verifier"),
        Err(error) => error,
    };
    assert_eq!(
        different_secret.kind(),
        std::io::ErrorKind::PermissionDenied
    );
}

#[test]
fn new_session_requires_the_established_store_secret() {
    let db = test_db();
    let store = PlanStore::new(&db).expect("init");
    store
        .bootstrap_approval_authority("trusted-session", [8; 32])
        .expect("initialize trusted store root");

    let error = match store.bootstrap_approval_authority("attacker-session", [9; 32]) {
        Ok(_) => panic!("a new session must not establish a different store secret"),
        Err(error) => error,
    };
    assert_eq!(error.kind(), std::io::ErrorKind::PermissionDenied);

    let trusted = store
        .bootstrap_approval_authority("second-trusted-session", [8; 32])
        .expect("the established store secret may authorize another session");
    store
        .validate_approval_authority(&trusted, "second-trusted-session")
        .expect("second session authority validates");
}

#[test]
fn concurrent_different_session_secrets_have_one_store_root_winner() {
    use std::sync::{Arc, Barrier};

    let db = Arc::new(test_db());
    let store = Arc::new(PlanStore::new(&db).expect("init"));
    let barrier = Arc::new(Barrier::new(2));
    let first_store = Arc::clone(&store);
    let first_barrier = Arc::clone(&barrier);
    let first = std::thread::spawn(move || {
        first_barrier.wait();
        first_store.bootstrap_approval_authority("root-session-a", [10; 32])
    });
    let second_store = Arc::clone(&store);
    let second_barrier = Arc::clone(&barrier);
    let second = std::thread::spawn(move || {
        second_barrier.wait();
        second_store.bootstrap_approval_authority("root-session-b", [11; 32])
    });

    let first = first.join().expect("first initializer thread");
    let second = second.join().expect("second initializer thread");
    assert!(
        first.is_ok() ^ second.is_ok(),
        "exactly one store root secret must initialize"
    );
}

#[test]
fn nonempty_authority_store_without_root_fails_closed() {
    let db = test_db();
    let store = PlanStore::new(&db).expect("init");
    let mut params = std::collections::BTreeMap::new();
    params.insert("sid".into(), cozo::DataValue::from("legacy-root-session"));
    params.insert("verifier".into(), cozo::DataValue::from("legacy-verifier"));
    db.run_script(
        "?[session_id, verifier] <- [[$sid, $verifier]] :insert plan_approval_authorities {session_id => verifier}",
        params,
        cozo::ScriptMutability::Mutable,
    )
    .expect("seed authority without store root");

    let error = match store.bootstrap_approval_authority("new-session", [12; 32]) {
        Ok(_) => panic!("a nonempty authority store without its root must fail closed"),
        Err(error) => error,
    };
    assert_eq!(error.kind(), std::io::ErrorKind::PermissionDenied);
    assert!(error.to_string().contains("root is missing"));
}

#[test]
fn authority_is_bound_to_its_session_and_store() {
    let db = test_db();
    let store = PlanStore::new(&db).expect("init");
    let authority = store
        .bootstrap_approval_authority("authority-session-a", [3; 32])
        .expect("initialize authority");
    let session_error = store
        .validate_approval_authority(&authority, "authority-session-b")
        .expect_err("authority must not cross session boundaries");
    assert_eq!(session_error.kind(), std::io::ErrorKind::PermissionDenied);

    let other_db = test_db();
    let other_store = PlanStore::new(&other_db).expect("init second store");
    let store_error = other_store
        .validate_approval_authority(&authority, "authority-session-a")
        .expect_err("authority must not cross durable store boundaries");
    assert_eq!(store_error.kind(), std::io::ErrorKind::PermissionDenied);
}

#[test]
fn concurrent_different_secret_initialization_has_one_winner() {
    use std::sync::{Arc, Barrier};

    let db = Arc::new(test_db());
    let store = Arc::new(PlanStore::new(&db).expect("init"));
    let barrier = Arc::new(Barrier::new(2));
    let first_store = Arc::clone(&store);
    let first_barrier = Arc::clone(&barrier);
    let first = std::thread::spawn(move || {
        first_barrier.wait();
        first_store.bootstrap_approval_authority("authority-concurrent", [4; 32])
    });
    let second_store = Arc::clone(&store);
    let second_barrier = Arc::clone(&barrier);
    let second = std::thread::spawn(move || {
        second_barrier.wait();
        second_store.bootstrap_approval_authority("authority-concurrent", [5; 32])
    });

    let first = first.join().expect("first initializer thread");
    let second = second.join().expect("second initializer thread");
    assert!(
        first.is_ok() ^ second.is_ok(),
        "exactly one secret must initialize"
    );
}

#[test]
fn concurrent_same_secret_initialization_authenticates_both_callers() {
    use std::sync::{Arc, Barrier};

    let db = Arc::new(test_db());
    let store = Arc::new(PlanStore::new(&db).expect("init"));
    let barrier = Arc::new(Barrier::new(2));
    let first_store = Arc::clone(&store);
    let first_barrier = Arc::clone(&barrier);
    let first = std::thread::spawn(move || {
        first_barrier.wait();
        first_store.bootstrap_approval_authority("authority-concurrent-same", [7; 32])
    });
    let second_store = Arc::clone(&store);
    let second_barrier = Arc::clone(&barrier);
    let second = std::thread::spawn(move || {
        second_barrier.wait();
        second_store.bootstrap_approval_authority("authority-concurrent-same", [7; 32])
    });

    assert!(first.join().expect("first initializer thread").is_ok());
    assert!(second.join().expect("second initializer thread").is_ok());
}

#[test]
fn authority_survives_physical_database_reopen_with_same_secret() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let path = directory.path().join("session.db");
    let secret = [6; 32];
    {
        let session_store = crate::storage::SessionStore::open(&path).expect("open session store");
        let store = PlanStore::new(session_store.db()).expect("init");
        store
            .bootstrap_approval_authority("authority-reopen", secret)
            .expect("initialize authority");
    }

    let session_store = crate::storage::SessionStore::open(&path).expect("reopen session store");
    let reopened = PlanStore::new(session_store.db()).expect("reopen plan store");
    let authority = reopened
        .bootstrap_approval_authority("authority-reopen", secret)
        .expect("authenticate with restart-stable secret");
    reopened
        .validate_approval_authority(&authority, "authority-reopen")
        .expect("reopened authority must validate");
}
