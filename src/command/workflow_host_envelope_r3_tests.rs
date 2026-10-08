use super::*;

async fn scalar_credentials(child: Child) {
    for secret in ["87654321", "-87654321.25", "true", "false"] {
        let ran = run_secret(child, secret).await;
        let result = ran
            .result
            .as_ref()
            .expect("sealed evidence stays parseable");
        assert!(
            clear_copies(ran.temp.path(), secret.as_bytes()).is_empty(),
            "scalar credential {secret} persisted"
        );
        let envelope = result.gate_envelope.clone().unwrap_or_else(|| {
            serde_json::from_slice::<archon_workflow::GateEnvelopeV1>(
                &std::fs::read(&ran.staged).unwrap(),
            )
            .unwrap()
        });
        assert_eq!(envelope.report["nested"][0], "[REDACTED]");
        assert_eq!(envelope.report["nested"][1]["pin"], "[REDACTED]");
        assert!(!serde_json::to_string(&envelope).unwrap().contains(secret));
    }
}
#[tokio::test]
async fn scalar_credentials_in_published_executor_evidence() {
    scalar_credentials(Child::Scalar).await;
}
#[tokio::test]
async fn scalar_credentials_in_operational_executor_evidence() {
    scalar_credentials(Child::ScalarOperational).await;
}
#[tokio::test]
async fn scalar_credentials_in_failed_executor_evidence() {
    scalar_credentials(Child::ScalarFailed).await;
}

#[tokio::test]
async fn child_supplied_numeric_version_cannot_hide_a_credential() {
    for secret in ["87654321", "87654322", "99998888"] {
        let ran = run_secret(Child::ScalarVersion, secret).await;
        assert!(
            clear_copies(ran.temp.path(), secret.as_bytes()).is_empty(),
            "version credential {secret} persisted"
        );
        let result = ran
            .result
            .as_ref()
            .expect("unsafe metadata produces operational evidence");
        assert!(result.publication_receipt.is_none());
        let envelope = result.gate_envelope.as_ref().unwrap();
        assert_eq!(
            envelope.schema_version,
            archon_workflow::GATE_ENVELOPE_SCHEMA_VERSION
        );
        assert!(envelope.operational_error.is_some());
    }
}
