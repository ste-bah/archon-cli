//! #201 Phase 3: the isolation backends gate on the declared class, not the
//! tool name, and all three agree.
//!
//! Driven through the public `SandboxBackend::check` rather than the internal
//! gate, so a backend that grew its own opinion again would fail here.

use archon_core::sandbox::{
    DockerConfig, DockerSandboxBackend, OpenShellConfig, OpenShellSandboxBackend, SshConfig,
    SshSandboxBackend,
};
use archon_permissions::{SandboxBackend, ToolCapability};

/// A binary whose `-V` and `--version` both succeed, guaranteed present because
/// it is running this test. The ssh and openshell backends probe their binary
/// before deciding anything, and a probe against the real `ssh`/`openshell`
/// would make these assertions depend on what happens to be installed.
fn probe_satisfying_binary() -> String {
    std::env::var("CARGO").unwrap_or_else(|_| "cargo".into())
}

fn backends() -> Vec<(&'static str, Box<dyn SandboxBackend>)> {
    vec![
        (
            "docker",
            Box::new(DockerSandboxBackend::new(
                DockerConfig {
                    enabled: true,
                    ..DockerConfig::default()
                },
                "rw",
            )),
        ),
        (
            "ssh",
            Box::new(SshSandboxBackend::new(SshConfig {
                enabled: true,
                binary: probe_satisfying_binary(),
                host: Some("sandbox.example".into()),
                workspace_mode: "mirror".into(),
                ..SshConfig::default()
            })),
        ),
        (
            "openshell",
            Box::new(OpenShellSandboxBackend::new(OpenShellConfig {
                enabled: true,
                binary: probe_satisfying_binary(),
                workspace_mode: "mirror".into(),
                ..OpenShellConfig::default()
            })),
        ),
    ]
}

/// The table the backends are supposed to implement, class by class.
const EXPECTED: [(ToolCapability, bool); 8] = [
    // Allowed by the gate so the backend's `terminal()` can answer: docker and
    // ssh can relocate a terminal, openshell cannot, and only they know which.
    (ToolCapability::TERMINAL, true),
    (ToolCapability::EXECUTION, true),
    (ToolCapability::FILE_READ, true),
    (ToolCapability::HostLocal, true),
    // Servable since Phase 2: each backend now has a filesystem of its own, so
    // a write lands in the world the shell sees rather than on the host.
    (ToolCapability::FILE_WRITE, true),
    (ToolCapability::HOST_HANDLE, false),
    (ToolCapability::Egress, false),
    (ToolCapability::ControlPlane, false),
];

#[test]
fn every_backend_implements_the_same_class_table() {
    for (label, backend) in backends() {
        for (capability, allowed) in EXPECTED {
            let result = backend.check("SomeTool", capability, &serde_json::json!({}));
            assert_eq!(
                result.is_ok(),
                allowed,
                "{label} got {result:?} for {}",
                capability.label()
            );
        }
    }
}

/// Bug 3 in the issue: the old `other => Err(...)` arm meant a tool nobody had
/// listed was denied under every backend, which is how the #190 terminal tools
/// shipped unusable. Nothing keys on the name now, so a name no backend has
/// ever heard of must be decided purely by its class.
#[test]
fn a_tool_no_backend_has_heard_of_is_decided_by_its_class() {
    for (label, backend) in backends() {
        backend
            .check(
                "ToolAddedNextWeek",
                ToolCapability::HostLocal,
                &serde_json::json!({}),
            )
            .unwrap_or_else(|error| panic!("{label} denied an unknown host-local tool: {error}"));

        let denial = backend
            .check(
                "ToolAddedNextWeek",
                ToolCapability::Egress,
                &serde_json::json!({}),
            )
            .expect_err("{label} allowed an unknown egress tool");
        assert!(
            !denial.contains("unsupported tool"),
            "{label} still refuses by name: {denial}"
        );
    }
}

/// Writes were refused because every backend installed the host filesystem, so
/// a write under a sandbox mutated the host while the session claimed
/// isolation. Phase 2 gave each backend a filesystem of its own — docker
/// translating against its bind mount, ssh and openshell reaching a remote
/// workspace over the transport `execute_bash` already uses — so the write now
/// lands in the world the shell sees.
#[test]
fn world_writes_are_served_now_that_each_backend_has_a_filesystem() {
    for (label, backend) in backends() {
        backend
            .check("Write", ToolCapability::FILE_WRITE, &serde_json::json!({}))
            .unwrap_or_else(|denial| panic!("{label} still refuses a world-bound write: {denial}"));
    }
}

/// The terminal tools, `PowerShell`, `lsp` and the CLI-backed evidence tools
/// all reach the world through a handle the backend cannot redirect. They were
/// denied before by falling off the end of the allowlist; now they are denied
/// for the reason that is actually true, which is what makes the denial
/// actionable.
///
/// Terminals used to be in this set and are not any more: `terminal()` can
/// relocate one, so they are decided there instead.
#[test]
fn host_handles_are_refused_with_the_reason_that_is_true() {
    for (label, backend) in backends() {
        let denial = backend
            .check("lsp", ToolCapability::HOST_HANDLE, &serde_json::json!({}))
            .expect_err("a host handle must not be allowed under isolation");

        assert!(denial.contains("outside the sandbox"), "{label}: {denial}");
    }
}

/// Preflight still wins: an unsafe or unusable backend refuses before the class
/// is ever consulted, including for classes the table allows.
#[test]
fn backend_preflight_still_precedes_the_class_decision() {
    let privileged = DockerSandboxBackend::new(
        DockerConfig {
            enabled: true,
            privileged: true,
            ..DockerConfig::default()
        },
        "rw",
    );

    for capability in [ToolCapability::EXECUTION, ToolCapability::Egress] {
        let denial = privileged
            .check("SomeTool", capability, &serde_json::json!({}))
            .expect_err("a privileged container must be refused whatever the class");

        // The class reason would be true too, for `Egress`. It must not be the
        // one reported: the container is the thing that is wrong, and telling
        // the caller about the tool instead sends them fixing the wrong thing.
        assert!(
            denial.contains("privileged"),
            "{} reported the class before the preflight: {denial}",
            capability.label()
        );
    }
}
