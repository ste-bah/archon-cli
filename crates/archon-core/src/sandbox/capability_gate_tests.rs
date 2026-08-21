use super::check_capability;
use archon_permissions::{ToolCapability, WorldReach};

/// Every class the enum can produce, so a variant added later without a gate
/// arm cannot slip past these tests by simply not being listed.
const ALL_CLASSES: [ToolCapability; 7] = [
    ToolCapability::WorldBound(WorldReach::Execution),
    ToolCapability::WorldBound(WorldReach::FileRead),
    ToolCapability::WorldBound(WorldReach::FileWrite),
    ToolCapability::WorldBound(WorldReach::HostHandle),
    ToolCapability::HostLocal,
    ToolCapability::Egress,
    ToolCapability::ControlPlane,
];

#[test]
fn execution_and_reads_and_host_local_state_are_servable() {
    for capability in [
        ToolCapability::EXECUTION,
        ToolCapability::FILE_READ,
        ToolCapability::HostLocal,
    ] {
        assert!(
            check_capability("docker", "SomeTool", capability).is_ok(),
            "{} should be servable",
            capability.label()
        );
    }
}

#[test]
fn world_writes_are_refused_until_the_world_has_a_filesystem() {
    let error = check_capability("docker", "Write", ToolCapability::FILE_WRITE).unwrap_err();

    assert!(
        error.contains("Write"),
        "denial must name the tool: {error}"
    );
    assert!(
        error.contains("Phase 2"),
        "denial must say what unblocks it: {error}"
    );
}

#[test]
fn host_handles_are_refused_because_they_would_run_outside_the_sandbox() {
    let error = check_capability("ssh", "TerminalCreate", ToolCapability::HOST_HANDLE).unwrap_err();

    assert!(error.contains("TerminalCreate"), "{error}");
    assert!(error.contains("outside the sandbox"), "{error}");
}

#[test]
fn egress_and_control_plane_stay_refused() {
    let egress = check_capability("openshell", "WebFetch", ToolCapability::Egress).unwrap_err();
    let control = check_capability("openshell", "Agent", ToolCapability::ControlPlane).unwrap_err();

    assert!(egress.contains("network"), "{egress}");
    assert!(control.contains("spawns or schedules"), "{control}");
}

/// The gate exists to stop the backends differing by accident. If one of them
/// ever grew its own opinion this would still pass, so the per-backend tests
/// assert the same table through the real `check` implementations.
#[test]
fn the_decision_never_depends_on_which_backend_asks() {
    for capability in ALL_CLASSES {
        let docker = check_capability("docker", "T", capability).is_ok();
        let ssh = check_capability("ssh", "T", capability).is_ok();
        let openshell = check_capability("openshell", "T", capability).is_ok();

        assert_eq!(
            docker,
            ssh,
            "{} differs between backends",
            capability.label()
        );
        assert_eq!(
            ssh,
            openshell,
            "{} differs between backends",
            capability.label()
        );
    }
}

/// The old catch-all denied any tool it did not recognise. Nothing keys on the
/// name any more, so an unknown name must be irrelevant to the outcome.
#[test]
fn an_unrecognised_tool_name_no_longer_decides_anything() {
    for capability in ALL_CLASSES {
        let known = check_capability("docker", "Read", capability).is_ok();
        let invented = check_capability("docker", "SomeToolAddedTomorrow", capability).is_ok();

        assert_eq!(
            known,
            invented,
            "{} outcome changed with the tool name",
            capability.label()
        );
    }
}
