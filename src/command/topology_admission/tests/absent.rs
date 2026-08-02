//! An absent or untracked tracker admits everything, and session state is
//! dropped at session end.

use super::super::*;
use super::*;

#[test]
fn no_installed_tracker_admits_everything() {
    let _guard = store_lock();
    uninstall();

    assert_eq!(
        admit(&request(
            "Bash",
            PermissionLevel::Dangerous,
            serde_json::json!({"command": "git push --force"})
        )),
        ToolRunAdmission::Allowed
    );
    assert!(active().is_none());
}

#[test]
fn admission_disabled_in_config_installs_no_tracker() {
    let _guard = store_lock();
    uninstall();
    let config = config_with(archon_core::config::TopologyConfig {
        admission_enabled: false,
        ..Default::default()
    });

    let installed = install(&config, SESSION);

    assert!(installed.is_none());
    assert!(active().is_none());
    assert_eq!(
        admit(&write_request("tu-1", "src/lib.rs")),
        ToolRunAdmission::Allowed
    );
}

#[test]
fn a_session_that_was_never_begun_admits_everything() {
    let _guard = store_lock();
    let config = config_with(archon_core::config::TopologyConfig::default());

    with_tracker(&config, || {
        let other = ToolRunAdmissionRequest {
            session_id: "some-other-session".into(),
            ..write_request("tu-1", "src/lib.rs")
        };
        assert_eq!(admit(&other), ToolRunAdmission::Allowed);
    });
}

#[test]
fn ending_a_session_drops_its_state() {
    let _guard = store_lock();
    let config = config_with(archon_core::config::TopologyConfig::default());

    with_tracker(&config, || {
        on_node_started(SESSION, "holder");
        let live = active().expect("installed");
        live.on_write_intent(
            SESSION,
            &archon_topology::live::WriteIntent {
                node_id: "holder".into(),
                paths: vec!["src/lib.rs".into()],
            },
        );
        assert!(matches!(
            admit(&write_request("tu-1", "src/lib.rs")),
            ToolRunAdmission::Blocked { .. }
        ));

        end_session(SESSION);

        assert_eq!(
            admit(&write_request("tu-2", "src/lib.rs")),
            ToolRunAdmission::Allowed
        );
    });
}
