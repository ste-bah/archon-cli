use super::*;

#[tokio::test]
async fn forwarded_short_credentials_reach_both_host_result_streams() {
    for secret in ["p4ss", "x", "é"] {
        let ran = run_secret(Child::JsonOutput, secret).await;
        let result = ran.result.unwrap();
        assert_eq!(result.stdout, "\"[REDACTED]\"", "{secret}");
        assert_eq!(result.stderr, "\"[REDACTED]\"", "{secret}");
    }
}

#[tokio::test]
async fn short_credentials_are_sealed_in_reports_and_typed_failures() {
    for child in [
        Child::Published,
        Child::TypedSecret,
        Child::OperationalTypedSecret,
    ] {
        let ran = run_secret(child, "p4ss").await;
        let result = ran.result.as_ref().unwrap();
        assert!(!result.stderr.contains("p4ss"));
        assert!(clear_copies(ran.temp.path(), b"p4ss").is_empty());
        if child != Child::Published {
            assert!(result.publication_receipt.is_none());
            assert!(result.gate_envelope.is_some());
        }
    }
}

#[tokio::test]
async fn credential_enum_collisions_preserve_executor_failure_evidence() {
    for secret in ["body", "report", "schema_version"] {
        let ran = run_secret(Child::TypedSecret, secret).await;
        let result = ran
            .result
            .as_ref()
            .expect("typed redaction must preserve the schema");
        assert!(result.publication_receipt.is_none());
        let finding = &result.gate_envelope.as_ref().unwrap().policy_findings[0];
        assert_eq!(finding.subject, "[REDACTED]");
        assert_eq!(
            finding.remediation_scope,
            archon_workflow::RemediationScope::Body
        );
    }
}

#[tokio::test]
async fn residual_redaction_refusal_preserves_an_operational_failure() {
    for secret in ["REDACTED", "[REDACTED]", "RED"] {
        let ran = run_secret(Child::TypedSecret, secret).await;
        let result = ran
            .result
            .as_ref()
            .expect("redaction refusal is a valid envelope");
        assert!(result.publication_receipt.is_none());
        let envelope = result.gate_envelope.as_ref().unwrap();
        assert!(envelope.operational_error.is_some() || !envelope.policy_findings.is_empty());
    }
}
