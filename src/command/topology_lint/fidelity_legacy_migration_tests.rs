use super::*;

fn convert_to_schema_four(path: &Path) {
    use sha2::Digest;

    let mut record: serde_json::Value =
        serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    let identity = &record["identity"];
    let legacy_identity = serde_json::json!({
        "binary": "binary-X",
        "model": identity["model"],
        "provider": identity["provider"],
        "request": identity["request"],
    });
    let material =
        serde_json::json!([4, record["obligation"], record["skeleton"], legacy_identity,]);
    let key = hex::encode(sha2::Sha256::digest(material.to_string().as_bytes()));
    record["schema"] = 4.into();
    record["key"] = key.clone().into();
    record["identity"] = legacy_identity;
    let legacy_path = path.with_file_name(format!("{key}.json"));
    std::fs::write(&legacy_path, serde_json::to_vec(&record).unwrap()).unwrap();
    std::fs::remove_file(path).unwrap();
}

#[tokio::test]
async fn matching_schema_four_records_reuse_without_critic_requests() {
    let temp = corpus();
    let cwd = temp.path();
    complete(cwd, Critic::new(), &FreezeResume::none()).await;
    let current = records(cwd);
    assert_eq!(current.len(), 6);
    for path in current {
        convert_to_schema_four(&path);
    }

    let critic = Critic::new();
    complete(cwd, critic.clone(), &FreezeResume::none()).await;
    assert_eq!(critic.calls(), 0, "validated legacy entries are promoted");
    assert_eq!(
        records(cwd).len(),
        12,
        "legacy and promoted records coexist"
    );
}
