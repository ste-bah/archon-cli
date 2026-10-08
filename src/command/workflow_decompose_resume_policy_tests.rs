//! A pre-binding fixed run (launched by a binary built before the Issue 349
//! check-policy binding) must pass resume admission. The fixture keeps the
//! key set and file layout of such a run, with generic project, PRD and path
//! placeholders: generated metadata, arguments, catalog, route, identity, and
//! the launch digest anchor in the older binary's form, which is the digest of
//! identity, arguments, catalog and route only. The test copies the fixture
//! into a temporary run directory and reads it back from there, as resume
//! reads a run.

use super::*;
use std::path::{Path, PathBuf};

const FIXTURE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/issue349_pre_binding_fixed_run"
);
const FILES: [&str; 6] = [
    "v2/generated-metadata.json",
    "decomposition/arguments.json",
    "decomposition/command-catalog.json",
    "decomposition/provider-route.json",
    "decomposition/state.json",
    "launch-digest-anchor.txt",
];

struct RunCopy {
    _dir: tempfile::TempDir,
    root: PathBuf,
}

impl RunCopy {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("run");
        for file in FILES {
            let target = root.join(file);
            std::fs::create_dir_all(target.parent().unwrap()).unwrap();
            std::fs::copy(Path::new(FIXTURE).join(file), &target).unwrap();
        }
        Self { _dir: dir, root }
    }

    fn json<T: serde::de::DeserializeOwned>(&self, file: &str) -> T {
        serde_json::from_slice(&std::fs::read(self.root.join(file)).unwrap()).unwrap()
    }

    fn anchor(&self) -> String {
        std::fs::read_to_string(self.root.join("launch-digest-anchor.txt")).unwrap()
    }

    fn admit(&self, metadata: &serde_json::Value) -> Result<Option<CheckPolicy>> {
        let state: archon_workflow::FixedDecompositionStateV1 =
            self.json("decomposition/state.json");
        let arguments: serde_json::Value = self.json("decomposition/arguments.json");
        admit(
            metadata,
            canonical_metadata(
                &state.identity,
                archon_workflow::workflow_scaffold_hash(
                    crate::command::workflow_decompose::FIXED_SCRIPT_SOURCE,
                ),
                &arguments,
            ),
            &state.identity,
            &arguments,
            &self.json("decomposition/command-catalog.json"),
            &self.json("decomposition/provider-route.json"),
            &self.anchor(),
        )
    }
}

#[test]
fn pre_binding_run_copy_is_admitted_with_no_check_policy() {
    let run = RunCopy::new();
    let metadata: serde_json::Value = run.json("v2/generated-metadata.json");
    assert!(metadata.get(CHECK_POLICY_KEY).is_none());
    // The other launch-bound gates resume applies hold for this run too.
    let state: archon_workflow::FixedDecompositionStateV1 = run.json("decomposition/state.json");
    assert_eq!(
        state.identity.script_digest,
        archon_workflow::workflow_scaffold_hash(
            crate::command::workflow_decompose::FIXED_SCRIPT_SOURCE
        )
    );
    let catalog: archon_workflow::CommandCapabilityCatalog =
        run.json("decomposition/command-catalog.json");
    assert_eq!(
        catalog,
        crate::command::workflow_host_command_catalog::fixed_decomposition_catalog(
            &state.identity.starting_binary_revision
        )
        .unwrap()
    );
    assert_eq!(run.admit(&metadata).unwrap(), None);
}

#[test]
fn pre_binding_run_copy_with_an_added_binding_is_refused() {
    let run = RunCopy::new();
    for binding in [
        serde_json::Value::Null,
        serde_json::json!({"toolchain_path": "/usr/bin:/bin", "bound": {}, "forwarded": []}),
    ] {
        let mut metadata: serde_json::Value = run.json("v2/generated-metadata.json");
        metadata[CHECK_POLICY_KEY] = binding;
        let error = run.admit(&metadata).unwrap_err().to_string();
        assert!(error.contains("launch snapshot differs"), "{error}");
    }
}

#[test]
fn pre_binding_run_copy_with_changed_metadata_or_anchor_is_refused() {
    let run = RunCopy::new();
    let mut metadata: serde_json::Value = run.json("v2/generated-metadata.json");
    metadata["script_lifecycle"] = serde_json::Value::Bool(false);
    let error = run.admit(&metadata).unwrap_err().to_string();
    assert!(error.contains("generated metadata differs"), "{error}");

    let metadata: serde_json::Value = run.json("v2/generated-metadata.json");
    std::fs::write(run.root.join("launch-digest-anchor.txt"), "0".repeat(64)).unwrap();
    let error = run.admit(&metadata).unwrap_err().to_string();
    assert!(error.contains("no check policy binding"), "{error}");
    assert!(error.contains("pre-binding form"), "{error}");
}
