use super::*;

#[tokio::test]
async fn ascii_json_output_is_redacted_at_the_host_result_boundary() {
    for secret in [
        "credential-é-canary",
        "credential-\u{007f}-canary",
        "credential-中-canary",
        "credential-😀-canary",
    ] {
        let ran = run_secret(Child::JsonOutput, secret).await;
        let result = ran.result.unwrap();
        assert_eq!(result.stdout, "\"[REDACTED]\"");
        assert_eq!(result.stderr, "\"[REDACTED]\"");
    }
}
