use archon_workflow::v2::{
    WorkflowV2Evidence, WorkflowV2EvidenceKind, WorkflowV2Harness, WorkflowV2HostCall,
    WorkflowV2HostMethod, WorkflowV2Result, WorkflowV2ResultStore, WorkflowV2Runtime,
    WorkflowV2Status,
};
use std::path::{Path, PathBuf};

#[test]
fn v2_boundary_exports_runtime_and_result_types() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = WorkflowV2ResultStore::new(temp.path());
    let _runtime = WorkflowV2Runtime::new(store);
    let harness = WorkflowV2Harness::new("export default async function workflow(w) {}");
    let call = WorkflowV2HostCall {
        id: "inspect".to_string(),
        method: WorkflowV2HostMethod::Agent,
        write_mode: None,
        options: Default::default(),
    };
    let mut result = WorkflowV2Result::accepted("boundary established");
    result.evidence.push(WorkflowV2Evidence::new(
        WorkflowV2EvidenceKind::Inspection,
        "boundary evidence",
    ));

    assert!(harness.source.contains("workflow"));
    assert_eq!(call.method, WorkflowV2HostMethod::Agent);
    assert_eq!(result.status, WorkflowV2Status::Accepted);
    assert_eq!(result.summary, "boundary established");
}

#[test]
fn v2_source_does_not_import_legacy_control_plane_modules() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src").join("v2");
    let files = rust_files(&src);
    assert!(
        !files.is_empty(),
        "expected V2 source files under {}",
        src.display()
    );

    let forbidden = [
        "crate::completion_proof",
        "crate::context_output",
        "crate::executor_output",
        "crate::fanout::",
        "crate::generated",
        "crate::item_filter",
        "crate::remediation_inventory",
        "crate::remediation_items",
        "crate::remediation_noop",
        "crate::required_artifact",
        "crate::work_unit_",
    ];

    let mut violations = Vec::new();
    for file in files {
        let body = std::fs::read_to_string(&file).expect("read V2 source");
        for needle in forbidden {
            if body.contains(needle) {
                violations.push(format!("{} imports {needle}", file.display()));
            }
        }
    }

    assert!(
        violations.is_empty(),
        "V2 must not depend on legacy YAML/artifact control-plane modules:\n{}",
        violations.join("\n")
    );
}

fn rust_files(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    collect_rust_files(root, &mut out);
    out.sort();
    out
}

fn collect_rust_files(path: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(path).expect("read source directory") {
        let entry = entry.expect("read source entry");
        let path = entry.path();
        if path.is_dir() {
            collect_rust_files(&path, out);
        } else if path.extension().and_then(|ext| ext.to_str()) == Some("rs") {
            out.push(path);
        }
    }
}
