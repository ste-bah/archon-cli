use super::{PlanStoreFactory, initialize_plan_authority, initialize_plan_store};

fn failing_plan_store_factory(
    _: &cozo::DbInstance,
) -> Result<archon_session::plan::PlanStore, std::io::Error> {
    Err(std::io::Error::other("injected plan store startup failure"))
}

#[test]
fn plan_authority_startup_initializes_before_store_attachment() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let database_path = directory.path().join("session.db");
    let secret_path = directory.path().join("plan-approval.secret");
    let session_store =
        archon_session::storage::SessionStore::open(&database_path).expect("session store");
    let plan_store =
        initialize_plan_store(session_store.db(), archon_session::plan::PlanStore::new)
            .expect("plan store");

    let authority = initialize_plan_authority(&plan_store, &secret_path, "startup-ordering")
        .expect("authority must initialize before attachment");
    plan_store
        .validate_approval_authority(&authority, "startup-ordering")
        .expect("authority is valid before any agent attachment or rehydration");
    assert!(secret_path.exists());
}

#[test]
fn plan_authority_startup_fails_closed_for_an_insecure_existing_secret() {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let directory = tempfile::tempdir().expect("temporary directory");
        let database_path = directory.path().join("session.db");
        let secret_path = directory.path().join("plan-approval.secret");
        std::fs::write(&secret_path, [7_u8; 32]).expect("write test secret");
        std::fs::set_permissions(&secret_path, std::fs::Permissions::from_mode(0o644))
            .expect("make secret insecure");
        let session_store =
            archon_session::storage::SessionStore::open(&database_path).expect("session store");
        let plan_store =
            initialize_plan_store(session_store.db(), archon_session::plan::PlanStore::new)
                .expect("plan store");

        let error = match initialize_plan_authority(&plan_store, &secret_path, "startup-insecure") {
            Ok(_) => panic!("insecure authority secret must stop startup"),
            Err(error) => error,
        };
        assert!(error.to_string().contains("owner-only"));
    }
}

#[test]
fn plan_store_startup_failure_fails_closed() {
    let db = cozo::DbInstance::new("mem", "", "").expect("in-memory Cozo database");
    let error = match initialize_plan_store(&db, failing_plan_store_factory as PlanStoreFactory) {
        Ok(_) => panic!("interactive initialization must reject unavailable plan persistence"),
        Err(error) => error,
    };
    assert!(
        error
            .to_string()
            .contains("failed to initialize plan store")
    );
    assert!(
        error
            .to_string()
            .contains("injected plan store startup failure")
    );
}
