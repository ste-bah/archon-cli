use super::{ToolCapability, WorldReach};

#[test]
fn world_bound_shorthands_name_the_reach_they_claim() {
    assert_eq!(
        ToolCapability::EXECUTION,
        ToolCapability::WorldBound(WorldReach::Execution)
    );
    assert_eq!(
        ToolCapability::FILE_READ,
        ToolCapability::WorldBound(WorldReach::FileRead)
    );
    assert_eq!(
        ToolCapability::FILE_WRITE,
        ToolCapability::WorldBound(WorldReach::FileWrite)
    );
    assert_eq!(
        ToolCapability::HOST_HANDLE,
        ToolCapability::WorldBound(WorldReach::HostHandle)
    );
}

/// Labels end up in denial messages the model reads, so two classes sharing a
/// label would make a refusal ambiguous about which rule fired.
#[test]
fn every_class_has_its_own_label() {
    let all = [
        ToolCapability::EXECUTION,
        ToolCapability::FILE_READ,
        ToolCapability::FILE_WRITE,
        ToolCapability::HOST_HANDLE,
        ToolCapability::HostLocal,
        ToolCapability::Egress,
        ToolCapability::ControlPlane,
    ];

    let mut labels: Vec<&str> = all.iter().map(|c| c.label()).collect();
    labels.sort_unstable();
    let unique = labels.len();
    labels.dedup();

    assert_eq!(labels.len(), unique, "two capability classes share a label");
}
