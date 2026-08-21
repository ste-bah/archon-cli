use super::check_capability;
use archon_permissions::{ToolCapability, WorldReach};

/// Every class the enum can produce, so a variant added later without a gate
/// arm cannot slip past these tests by simply not being listed.
const ALL_CLASSES: [ToolCapability; 8] = [
    ToolCapability::WorldBound(WorldReach::Terminal),
    ToolCapability::WorldBound(WorldReach::Execution),
    ToolCapability::WorldBound(WorldReach::FileRead),
    ToolCapability::WorldBound(WorldReach::FileWrite),
    ToolCapability::WorldBound(WorldReach::HostHandle),
    ToolCapability::HostLocal,
    ToolCapability::Egress,
    ToolCapability::ControlPlane,
];

#[test]
fn execution_and_files_and_host_local_state_are_servable() {
    for capability in [
        ToolCapability::EXECUTION,
        ToolCapability::FILE_READ,
        ToolCapability::FILE_WRITE,
        ToolCapability::HostLocal,
    ] {
        assert!(
            check_capability("docker", "SomeTool", capability).is_ok(),
            "{} should be servable",
            capability.label()
        );
    }
}

/// The reason writes were refused was that every backend installed the host
/// filesystem, so a write under a sandbox mutated the host. Phase 2 gave each
/// backend its own world, and this is what that bought.
#[test]
fn world_writes_are_served_now_that_each_backend_has_a_filesystem() {
    for backend in ["docker", "ssh", "openshell"] {
        assert!(
            check_capability(backend, "Write", ToolCapability::FILE_WRITE).is_ok(),
            "{backend} has a filesystem of its own, so a write lands in its world"
        );
    }
}

#[test]
fn host_handles_are_refused_because_they_would_run_outside_the_sandbox() {
    let error = check_capability("ssh", "lsp", ToolCapability::HOST_HANDLE).unwrap_err();

    assert!(error.contains("lsp"), "{error}");
    assert!(error.contains("outside the sandbox"), "{error}");
}

/// A terminal is not a host handle any more: `SandboxBackend::terminal` can
/// relocate it. The gate lets it through so the backend can give the answer
/// that is true for it — refusing here would refuse docker and ssh, which can.
#[test]
fn terminals_reach_the_backend_rather_than_being_decided_by_the_gate() {
    for backend in ["docker", "ssh", "openshell"] {
        assert!(
            check_capability(backend, "TerminalCreate", ToolCapability::TERMINAL).is_ok(),
            "{backend} must decide a terminal in terminal(), not here"
        );
    }
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
