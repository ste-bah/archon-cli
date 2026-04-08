/// TASK-HOOK-022: Persistent Permission Updates tests
use std::sync::Mutex;

use archon_core::hooks::{
    PermissionStore, PermissionUpdate, PermissionUpdateDestination, SourceAuthority,
    apply_permission_updates,
};

/// Mock permission store that records all operations
struct MockStore {
    operations: Mutex<Vec<String>>,
}

impl MockStore {
    fn new() -> Self {
        Self {
            operations: Mutex::new(Vec::new()),
        }
    }
    fn ops(&self) -> Vec<String> {
        self.operations.lock().unwrap().clone()
    }
}

impl PermissionStore for MockStore {
    fn add_rules(
        &self,
        dest: &PermissionUpdateDestination,
        rules: &[String],
    ) -> Result<(), String> {
        self.operations
            .lock()
            .unwrap()
            .push(format!("add_rules:{dest:?}:{rules:?}"));
        Ok(())
    }
    fn replace_rules(
        &self,
        dest: &PermissionUpdateDestination,
        rules: &[String],
    ) -> Result<(), String> {
        self.operations
            .lock()
            .unwrap()
            .push(format!("replace_rules:{dest:?}:{rules:?}"));
        Ok(())
    }
    fn remove_rules(
        &self,
        dest: &PermissionUpdateDestination,
        rules: &[String],
    ) -> Result<(), String> {
        self.operations
            .lock()
            .unwrap()
            .push(format!("remove_rules:{dest:?}:{rules:?}"));
        Ok(())
    }
    fn set_mode(&self, dest: &PermissionUpdateDestination, mode: &str) -> Result<(), String> {
        self.operations
            .lock()
            .unwrap()
            .push(format!("set_mode:{dest:?}:{mode}"));
        Ok(())
    }
    fn add_directories(
        &self,
        dest: &PermissionUpdateDestination,
        dirs: &[String],
    ) -> Result<(), String> {
        self.operations
            .lock()
            .unwrap()
            .push(format!("add_directories:{dest:?}:{dirs:?}"));
        Ok(())
    }
    fn remove_directories(
        &self,
        dest: &PermissionUpdateDestination,
        dirs: &[String],
    ) -> Result<(), String> {
        self.operations
            .lock()
            .unwrap()
            .push(format!("remove_directories:{dest:?}:{dirs:?}"));
        Ok(())
    }
}

#[test]
fn test_permission_update_types_exist() {
    let _ = PermissionUpdateDestination::UserSettings;
    let _ = PermissionUpdateDestination::ProjectSettings;
    let _ = PermissionUpdateDestination::LocalSettings;
    let _ = PermissionUpdateDestination::Session;

    let _ = PermissionUpdate::AddRules {
        destination: PermissionUpdateDestination::Session,
        rules: vec![],
    };
    let _ = PermissionUpdate::ReplaceRules {
        destination: PermissionUpdateDestination::Session,
        rules: vec![],
    };
    let _ = PermissionUpdate::RemoveRules {
        destination: PermissionUpdateDestination::Session,
        rules: vec![],
    };
    let _ = PermissionUpdate::SetMode {
        destination: PermissionUpdateDestination::Session,
        mode: "ask".into(),
    };
    let _ = PermissionUpdate::AddDirectories {
        destination: PermissionUpdateDestination::Session,
        directories: vec![],
    };
    let _ = PermissionUpdate::RemoveDirectories {
        destination: PermissionUpdateDestination::Session,
        directories: vec![],
    };
}

#[test]
fn test_apply_add_rules() {
    let store = MockStore::new();
    let updates = vec![PermissionUpdate::AddRules {
        destination: PermissionUpdateDestination::LocalSettings,
        rules: vec!["allow:Bash".into(), "allow:Read".into()],
    }];
    let errors = apply_permission_updates(&updates, &SourceAuthority::Project, &store);
    assert!(errors.is_empty());
    let ops = store.ops();
    assert_eq!(ops.len(), 1);
    assert!(ops[0].contains("add_rules"));
    assert!(ops[0].contains("LocalSettings"));
}

#[test]
fn test_policy_required_for_user_settings() {
    let store = MockStore::new();
    let updates = vec![PermissionUpdate::AddRules {
        destination: PermissionUpdateDestination::UserSettings,
        rules: vec!["allow:*".into()],
    }];
    // Non-policy (Project) trying to write to UserSettings -> should be rejected
    let errors = apply_permission_updates(&updates, &SourceAuthority::Project, &store);
    assert!(errors.is_empty()); // no error, just silently dropped
    assert!(
        store.ops().is_empty(),
        "non-policy hook should not write to UserSettings"
    );
}

#[test]
fn test_policy_can_write_user_settings() {
    let store = MockStore::new();
    let updates = vec![PermissionUpdate::AddRules {
        destination: PermissionUpdateDestination::UserSettings,
        rules: vec!["deny:Bash(rm *)".into()],
    }];
    let errors = apply_permission_updates(&updates, &SourceAuthority::Policy, &store);
    assert!(errors.is_empty());
    assert_eq!(
        store.ops().len(),
        1,
        "policy hook should write to UserSettings"
    );
}

#[test]
fn test_session_destination_in_memory() {
    let store = MockStore::new();
    let updates = vec![PermissionUpdate::SetMode {
        destination: PermissionUpdateDestination::Session,
        mode: "auto".into(),
    }];
    let errors = apply_permission_updates(&updates, &SourceAuthority::User, &store);
    assert!(errors.is_empty());
    let ops = store.ops();
    assert_eq!(ops.len(), 1);
    assert!(ops[0].contains("Session"));
    assert!(ops[0].contains("auto"));
}

#[test]
fn test_all_6_variants_apply() {
    let store = MockStore::new();
    let updates = vec![
        PermissionUpdate::AddRules {
            destination: PermissionUpdateDestination::LocalSettings,
            rules: vec!["r1".into()],
        },
        PermissionUpdate::ReplaceRules {
            destination: PermissionUpdateDestination::LocalSettings,
            rules: vec!["r2".into()],
        },
        PermissionUpdate::RemoveRules {
            destination: PermissionUpdateDestination::LocalSettings,
            rules: vec!["r3".into()],
        },
        PermissionUpdate::SetMode {
            destination: PermissionUpdateDestination::Session,
            mode: "deny".into(),
        },
        PermissionUpdate::AddDirectories {
            destination: PermissionUpdateDestination::Session,
            directories: vec!["/tmp".into()],
        },
        PermissionUpdate::RemoveDirectories {
            destination: PermissionUpdateDestination::Session,
            directories: vec!["/var".into()],
        },
    ];
    let errors = apply_permission_updates(&updates, &SourceAuthority::Local, &store);
    assert!(errors.is_empty());
    assert_eq!(store.ops().len(), 6, "all 6 variants should be applied");
}
