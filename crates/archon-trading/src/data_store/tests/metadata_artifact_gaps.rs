use super::*;

type MetadataFault = (&'static str, fn(&mut DatasetMetadata));

const REQUIRED_METADATA_FAULTS: &[MetadataFault] = &[
    ("schema_version", |metadata| metadata.schema_version.clear()),
    ("dataset_id", |metadata| metadata.dataset_id.clear()),
    ("version", |metadata| metadata.version.clear()),
    ("canonical_instrument", |metadata| {
        metadata.canonical_instrument.clear()
    }),
    ("asset_class", |metadata| metadata.asset_class.clear()),
    ("provider", |metadata| metadata.provider.clear()),
    ("provider_symbol", |metadata| {
        metadata.provider_symbol.clear()
    }),
    ("timeframe", |metadata| metadata.timeframe.clear()),
    ("price_basis", |metadata| metadata.price_basis.clear()),
    ("session", |metadata| metadata.session.clear()),
    ("symbol_map", |metadata| metadata.symbol_map.clear()),
    ("timezone", |metadata| metadata.timezone.clear()),
    ("adjustment", |metadata| metadata.adjustment.clear()),
    ("license", |metadata| metadata.license.clear()),
    ("coverage.start", |metadata| metadata.coverage.start.clear()),
    ("coverage.end", |metadata| metadata.coverage.end.clear()),
    ("coverage.expected_bars", |metadata| {
        metadata.coverage.expected_bars = 0
    }),
    ("gaps.expected_bars", |metadata| {
        metadata.gaps.expected_bars = 0
    }),
    ("checksum", |metadata| metadata.checksum.clear()),
    ("checksums.raw_sha256", |metadata| {
        metadata.checksums.raw_sha256.clear()
    }),
    ("checksums.normalized_sha256", |metadata| {
        metadata.checksums.normalized_sha256.clear()
    }),
    ("checksums.metadata_sha256", |metadata| {
        metadata.checksums.metadata_sha256.clear()
    }),
    ("source.license_notes", |metadata| {
        metadata.source.license_notes.clear()
    }),
    ("source.url_or_endpoint", |metadata| {
        metadata.source.url_or_endpoint.clear()
    }),
    ("source.retrieved_at", |metadata| {
        metadata.source.retrieved_at.clear()
    }),
    ("quality_status", |metadata| metadata.quality_status.clear()),
    ("created_at", |metadata| metadata.created_at.clear()),
];

const REQUIRED_PATH_FAULTS: &[MetadataFault] = &[
    ("paths.raw", |metadata| metadata.paths.raw.clear()),
    ("paths.raw_response", |metadata| {
        metadata.paths.raw_response.clear()
    }),
    ("paths.raw_request", |metadata| {
        metadata.paths.raw_request.clear()
    }),
    ("paths.redacted_headers", |metadata| {
        metadata.paths.redacted_headers.clear()
    }),
    ("paths.provider_notes", |metadata| {
        metadata.paths.provider_notes.clear()
    }),
    ("paths.normalized", |metadata| {
        metadata.paths.normalized.clear()
    }),
    ("paths.validation", |metadata| {
        metadata.paths.validation.clear()
    }),
    ("paths.manifest", |metadata| metadata.paths.manifest.clear()),
];

pub(super) fn run() {
    let bars = fixture_bars();
    assert_complete_metadata_faults(&bars);
    assert_artifact_path_faults(&bars);
    assert_stale_normalized_checksums(&bars);
    assert_filesystem_artifact_faults(&bars);
}

fn fixture_bars() -> Vec<OhlcvBar> {
    vec![
        bar("2026-01-01T00:00:00Z", 10.0, 100.0),
        bar("2026-01-02T00:00:00Z", 11.0, 110.0),
    ]
}

fn assert_complete_metadata_faults(bars: &[OhlcvBar]) {
    let baseline = complete_metadata(bars);
    assert_passed_check(&report(&baseline, bars), "metadata.complete", "baseline");
    for (case, mutate) in REQUIRED_METADATA_FAULTS {
        let mut metadata = baseline.clone();
        mutate(&mut metadata);
        assert_failed_check_for_case(&report(&metadata, bars), "metadata.complete", case);
    }
}

fn assert_artifact_path_faults(bars: &[OhlcvBar]) {
    let baseline = complete_metadata(bars);
    assert_passed_check(
        &report(&baseline, bars),
        "artifact.required_files",
        "baseline",
    );
    for (case, mutate) in REQUIRED_PATH_FAULTS {
        let mut metadata = baseline.clone();
        mutate(&mut metadata);
        let result = report(&metadata, bars);
        assert_failed_check_for_case(&result, "metadata.complete", case);
        assert_failed_check_for_case(&result, "artifact.required_files", case);
    }
    for (case, unsafe_path) in [("absolute", "/tmp/escape"), ("traversal", "../escape")] {
        let mut metadata = baseline.clone();
        metadata.paths.raw_response = unsafe_path.into();
        assert_failed_check_for_case(&report(&metadata, bars), "artifact.required_files", case);
    }
}

fn assert_stale_normalized_checksums(bars: &[OhlcvBar]) {
    for field in ["checksum", "checksums.normalized_sha256"] {
        let mut metadata = complete_metadata(bars);
        if field == "checksum" {
            metadata.checksum = "stale".into();
        } else {
            metadata.checksums.normalized_sha256 = "stale".into();
        }
        assert_failed_check_for_case(
            &report(&metadata, bars),
            "artifact.normalized_checksum",
            field,
        );
    }
}

fn assert_filesystem_artifact_faults(bars: &[OhlcvBar]) {
    let temp = tempfile::tempdir().unwrap();
    let metadata = complete_metadata(bars);
    write_required_files(temp.path(), &metadata);
    let baseline =
        validation_report_at_root(temp.path(), &metadata, bars, metadata.created_at.clone());
    assert_passed_check(&baseline, "artifact.required_files", "filesystem baseline");

    let missing = temp.path().join(&metadata.paths.raw_request);
    std::fs::remove_file(missing).unwrap();
    assert_root_artifact_failure(temp.path(), &metadata, bars, "missing");
    write_required_files(temp.path(), &metadata);

    let response = temp.path().join(&metadata.paths.raw_response);
    std::fs::remove_file(&response).unwrap();
    std::fs::create_dir(&response).unwrap();
    assert_root_artifact_failure(temp.path(), &metadata, bars, "directory");

    assert_symlink_escape_is_rejected(temp.path(), &metadata, bars);
}

fn write_required_files(root: &Path, metadata: &DatasetMetadata) {
    for relative in [
        &metadata.paths.raw,
        &metadata.paths.raw_response,
        &metadata.paths.raw_request,
        &metadata.paths.redacted_headers,
        &metadata.paths.provider_notes,
        &metadata.paths.normalized,
    ] {
        let path = root.join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        if !path.is_dir() {
            std::fs::write(path, b"fixture").unwrap();
        }
    }
}

#[cfg(unix)]
fn assert_symlink_escape_is_rejected(root: &Path, metadata: &DatasetMetadata, bars: &[OhlcvBar]) {
    use std::os::unix::fs::symlink;

    let outside = tempfile::NamedTempFile::new().unwrap();
    let request = root.join(&metadata.paths.raw_request);
    std::fs::remove_file(&request).unwrap();
    symlink(outside.path(), request).unwrap();
    assert_root_artifact_failure(root, metadata, bars, "symlink escape");
}

#[cfg(not(unix))]
fn assert_symlink_escape_is_rejected(_: &Path, _: &DatasetMetadata, _: &[OhlcvBar]) {}

fn assert_root_artifact_failure(
    root: &Path,
    metadata: &DatasetMetadata,
    bars: &[OhlcvBar],
    case: &str,
) {
    let result = validation_report_at_root(root, metadata, bars, metadata.created_at.clone());
    assert_failed_check_for_case(&result, "artifact.required_files", case);
}

fn report(metadata: &DatasetMetadata, bars: &[OhlcvBar]) -> ValidationReport {
    validation_report(metadata, bars, metadata.created_at.clone())
}

fn assert_passed_check(report: &ValidationReport, id: &str, case: &str) {
    assert!(
        report
            .checks
            .iter()
            .any(|check| check.id == id && check.status == ValidationStatus::Passed),
        "{case}: expected passed check {id}; checks were {:?}",
        report.checks
    );
}

fn assert_failed_check_for_case(report: &ValidationReport, id: &str, case: &str) {
    assert!(
        report
            .checks
            .iter()
            .any(|check| check.id == id && check.status == ValidationStatus::Failed),
        "{case}: expected failed check {id}; checks were {:?}",
        report.checks
    );
}
