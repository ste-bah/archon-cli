use super::*;

#[cfg(unix)]
#[tokio::test]
async fn sealing_failure_restores_access_and_removes_secret_bearing_staging_tree() {
    for child in [
        Child::UnreadableStaging,
        Child::UnreadableNestedStaging,
        Child::UnreadableDeepStaging,
    ] {
        let ran = run(child).await;
        assert!(
            ran.result.is_err(),
            "the child result is refused after sealing fails"
        );
        let staging_root = ran.staged.parent().unwrap();
        assert!(
            !staging_root.exists(),
            "the whole staging tree must be removed after access is restored: {}",
            staging_root.display()
        );
        assert_no_clear_copy(&ran);
    }
}

#[tokio::test]
async fn round2_json_escaped_credentials_are_sealed() {
    for secret in [
        "credential-quote\"-canary",
        "credential-backslash\\-canary",
        "credential-newline\n-canary",
    ] {
        let ran = run_secret(Child::Published, secret).await;
        ran.result.as_ref().unwrap();
        let escaped = serde_json::to_string(secret).unwrap();
        for needle in [secret.as_bytes(), &escaped.as_bytes()[1..escaped.len() - 1]] {
            assert!(
                clear_copies(ran.temp.path(), needle).is_empty(),
                "{secret:?}"
            );
        }
    }
}

#[tokio::test]
async fn round2_object_keys_are_sealed() {
    let ran = run(Child::SecretKey).await;
    ran.result.as_ref().unwrap();
    assert_no_clear_copy(&ran);
    let value: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&ran.envelope).unwrap()).unwrap();
    assert_eq!(value["report"]["[REDACTED]"], "failure");
}

#[tokio::test]
async fn round2_interrupted_temporary_envelopes_are_removed() {
    for child in [Child::Interrupted, Child::Paused, Child::Cancelled] {
        let ran = run(child).await;
        assert!(ran.result.is_err());
        assert_no_clear_copy(&ran);
    }
}
