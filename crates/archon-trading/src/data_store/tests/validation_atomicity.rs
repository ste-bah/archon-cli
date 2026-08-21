use super::*;

const PRIOR: [(&str, &[u8]); 3] = [
    ("validation.json", b"prior-validation"),
    ("metadata.json", b"prior-metadata"),
    (
        "registry.json",
        b"{\"schema\":\"archon-trading-data-registry-v1\",\"datasets\":{},\"snapshots\":{},\"last_updated\":\"2026-01-01T00:00:00Z\"}",
    ),
];

const OPERATIONS: [&str; 6] = [
    "temp.create",
    "temp.write",
    "temp.flush",
    "temp.file_sync",
    "replace",
    "directory_sync",
];

pub(super) fn run() {
    let boundaries = failure_boundaries();
    let unique: std::collections::BTreeSet<_> = boundaries.iter().collect();
    assert_eq!(boundaries.len(), PRIOR.len() * OPERATIONS.len());
    assert_eq!(unique.len(), boundaries.len());
    for boundary in boundaries {
        assert_boundary_is_atomic(&boundary);
    }
}

fn failure_boundaries() -> Vec<String> {
    (0..PRIOR.len())
        .flat_map(|index| {
            OPERATIONS
                .iter()
                .map(move |operation| format!("transaction.{operation}.{index}"))
        })
        .collect()
}

fn assert_boundary_is_atomic(boundary: &str) {
    let temp = tempfile::tempdir().unwrap();
    for (name, bytes) in PRIOR {
        std::fs::write(temp.path().join(name), bytes).unwrap();
    }
    let registry_before = registry_health(temp.path());

    inject_io_failure(Some(boundary));
    let result = atomic_write_many(
        PRIOR
            .iter()
            .map(|(name, _)| (temp.path().join(name), format!("new-{name}").into_bytes()))
            .collect(),
    );
    inject_io_failure(None);

    assert!(result.is_err(), "{boundary} did not fail");
    for (name, expected) in PRIOR {
        assert_eq!(
            std::fs::read(temp.path().join(name)).unwrap(),
            expected,
            "{boundary}: {name}"
        );
    }
    assert_eq!(registry_health(temp.path()), registry_before, "{boundary}");
    assert_no_staged_files(temp.path(), boundary);
}

fn registry_health(root: &Path) -> (bool, String, usize) {
    let path = root.join("registry.json");
    let registry: PersistentDatasetRegistry = read_json(&path).unwrap();
    (
        path.is_file(),
        registry.schema_version,
        registry.datasets.len(),
    )
}

fn assert_no_staged_files(root: &Path, boundary: &str) {
    let staged: Vec<_> = std::fs::read_dir(root)
        .unwrap()
        .filter_map(Result::ok)
        .map(|entry| entry.file_name())
        .filter(|name| name.to_string_lossy().ends_with(".tmp"))
        .collect();
    assert!(
        staged.is_empty(),
        "{boundary}: staged files remain: {staged:?}"
    );
}
