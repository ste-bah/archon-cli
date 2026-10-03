use super::*;

#[test]
fn each_batch_uses_its_rendered_request_and_unknown_requests_never_reuse() {
    let temp = tempfile::tempdir().unwrap();
    let identity = StoreIdentity {
        binary: "build".into(),
        provider: Some("provider".into()),
        model: "model".into(),
        request: Some("representative-request".into()),
    };
    let store = VerdictStore::new(temp.path().join("verdicts"), identity);
    let first = store.for_request(Some("rendered-template-one".into()));
    assert!(first.save("same-input-content", &[]).unwrap());
    assert_eq!(first.load("same-input-content", &[], &[]), Some(vec![]));
    let changed = store.for_request(Some("rendered-template-two".into()));
    assert_eq!(changed.load("same-input-content", &[], &[]), None);
    let unknown = store.for_request(None);
    assert_eq!(unknown.load("same-input-content", &[], &[]), None);
    assert!(!unknown.save("unknown-request", &[]).unwrap());
    assert_eq!(std::fs::read_dir(store.dir()).unwrap().count(), 1);
}
