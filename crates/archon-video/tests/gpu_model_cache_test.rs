use archon_policy::VideoPolicy;

use archon_video::asr::{probe_gpu_backend, resolve_model_path};
use archon_video::errors::VideoError;

#[test]
fn cuda_probe_falls_back_to_cpu_on_ci() {
    let (device, fallback) = probe_gpu_backend("cuda");

    assert_eq!(device, "cpu");
    assert!(fallback);
}

#[test]
fn model_source_existing_file_resolves_without_download() {
    let dir = tempfile::tempdir().unwrap();
    let model = dir.path().join("fixture.bin");
    std::fs::write(&model, b"model").unwrap();
    let policy = VideoPolicy::default();

    let resolved = resolve_model_path(&model.display().to_string(), "", "base", &policy).unwrap();

    assert_eq!(resolved, model);
}

#[test]
fn missing_cached_model_requires_network_policy() {
    let dir = tempfile::tempdir().unwrap();
    let policy = VideoPolicy {
        allow_cloud_asr: false,
        ..VideoPolicy::default()
    };

    let err =
        resolve_model_path("", &dir.path().display().to_string(), "base", &policy).unwrap_err();

    assert!(matches!(err, VideoError::PolicyDenied { .. }));
}
