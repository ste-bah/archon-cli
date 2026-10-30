use super::*;
use crate::context_window::RequestPressureKind;
use crate::provider::LlmError;

fn observed() -> chrono::DateTime<chrono::Utc> {
    chrono::DateTime::parse_from_rfc3339("2026-10-29T00:00:00Z")
        .unwrap()
        .with_timezone(&chrono::Utc)
}

#[test]
fn r3_prefix_keeps_raw_pressure() {
    let raw = DiagnosticMessage::from("Authorization: Bearer token; tool_result maximum length");
    let prefixed = LlmError::Http(raw.with_prefix("context window: "));
    assert_eq!(
        prefixed.request_pressure_kind(),
        Some(RequestPressureKind::ToolResultField)
    );
    // 86fbab995 stored a rendering itself, so it is classified from then on.
    let bounded = raw.clone().map_text(|_| "safe diagnostic".into());
    assert_eq!(
        LlmError::Http(bounded.with_prefix("context window: ")).request_pressure_kind(),
        Some(RequestPressureKind::AggregateContext)
    );
}

#[test]
fn r3_prefix_keeps_raw_reset() {
    let raw = DiagnosticMessage::from(
        "Authorization: Bearer token; 2026-11-01T00:00:00Z\nobserved 2026-10-28T00:00:00Z",
    );
    let november = chrono::DateTime::parse_from_rfc3339("2026-11-01T00:00:00Z")
        .unwrap()
        .with_timezone(&chrono::Utc);
    assert_eq!(
        raw.with_prefix("").reset_hint_at(observed()),
        Some(november)
    );
    let bounded = raw
        .clone()
        .map_text(|_| "observed 2026-10-28T00:00:00Z".into());
    let october = chrono::DateTime::parse_from_rfc3339("2026-10-28T00:00:00Z")
        .unwrap()
        .with_timezone(&chrono::Utc);
    assert_eq!(
        bounded.with_prefix("").reset_hint_at(observed()),
        Some(october)
    );
}

#[test]
fn r6_prefix_status_matches_main() {
    // 86fbab995 classified the composed text: a non-empty prefix leads it.
    let raw = DiagnosticMessage::from("503 service unavailable");
    assert_eq!(raw.with_prefix("400: ").leading_status(), Some(400));
    assert_eq!(raw.with_prefix("").leading_status(), Some(503));
    let rejected = DiagnosticMessage::from("401 rejected").with_prefix("503: ");
    assert_eq!(rejected.leading_status(), Some(503));
    assert!(
        LlmError::Http(rejected).is_operational(),
        "main read status 503 first"
    );
}

#[test]
fn r3_stream_event_round_trip_keeps_facts_and_reads_legacy() {
    use crate::streaming::StreamEvent;
    let event = StreamEvent::Error {
        error_type: "protocol".into(),
        message: "sk-ant-abcdefghijklmnopqrstuvwxyz123456; stream ended before message_stop".into(),
    };
    let json = serde_json::to_string(&event).unwrap();
    assert!(!json.contains("sk-ant-abcdef"));
    let restored: StreamEvent = serde_json::from_str(&json).unwrap();
    let StreamEvent::Error { message, .. } = restored else {
        panic!("wrong event")
    };
    assert!(
        message.is_incomplete_stream(),
        "round trip lost raw incomplete-stream fact"
    );
    let legacy: DiagnosticMessage =
        serde_json::from_str(r#""stream ended before message_stop""#).unwrap();
    assert!(legacy.is_incomplete_stream());
}

#[test]
fn r9_round_trip_classifies_the_stored_text() {
    // Stored text is redacted; it is classified as stored, never "restored".
    let raw = "Authorization: Bearer secret-token-123; rate limit 429";
    let json = serde_json::to_string(&DiagnosticMessage::from(raw)).unwrap();
    assert!(!json.contains("secret-token-123"), "{json}");
    let restored: DiagnosticMessage = serde_json::from_str(&json).unwrap();
    let stored: String = serde_json::from_str(&json).unwrap();
    assert_eq!(restored.raw(), stored);
    assert_eq!(restored.as_str(), stored);
    // Records written by earlier builds carried a `facts` object; it is ignored.
    let earlier: DiagnosticMessage =
        serde_json::from_str(r#"{"text":"plain","facts":{"rate_limited":true}}"#).unwrap();
    assert_eq!(earlier.as_str(), "plain");
    assert!(!earlier.is_rate_limited());
}

#[test]
fn r6_prefix_operational_matches_main() {
    // Expected values are main's classification of the composed text; the
    // C0 stream rows check the same compositions against 86fbab995's code.
    let operational = |message: DiagnosticMessage| LlmError::Http(message).is_operational();
    for body in [
        "protocol: connection reset",
        "parse_error: connection reset",
    ] {
        let raw = DiagnosticMessage::from(body);
        assert!(!operational(raw.clone()));
        // A prefix moves the leading exclusion away; "connection" then matches.
        for prefix in ["peer: ", "connection: ", "503: "] {
            assert!(operational(raw.with_prefix(prefix)), "{prefix}{body}");
        }
    }
    let rejected = DiagnosticMessage::from("401 rejected");
    assert!(!operational(rejected.with_prefix("peer: ")));
    assert!(operational(rejected.with_prefix("connection: ")));
    assert!(operational(rejected.with_prefix("503: ")));
    // 86fbab995 stored the rendering, so composing it classifies the rendering.
    let bounded = DiagnosticMessage::from("unclassified failure")
        .map_text(|_| "connection reset rate limit".into());
    assert!(operational(bounded.with_prefix("peer: ")));
    assert!(bounded.with_prefix("peer: ").is_rate_limited());
    assert!(operational(
        DiagnosticMessage::from("unclassified failure").with_prefix("connection: ")
    ));
    assert!(!operational(
        DiagnosticMessage::from("connection reset").with_prefix("protocol: ")
    ));
}

#[test]
fn r6_stream_prefix_matches_main_operational() {
    // 86fbab995 composed "protocol: connection reset" before classifying it.
    let fault = crate::provider_fault::from_stream_error("protocol", "connection reset");
    assert!(!fault.is_operational(), "{fault}");
}

#[test]
fn r6_rejected_type_and_message_join_like_main() {
    // 86fbab995 joined error_type and message with one space before matching.
    let fault = LlmError::Rejected {
        error_type: "context".into(),
        message: "window exceeded".into(),
    };
    assert!(fault.is_context_window_exceeded());
    assert_eq!(
        fault.request_pressure_kind(),
        Some(RequestPressureKind::AggregateContext)
    );
}

#[test]
fn r7_reset_hint_composes_like_the_joined_text() {
    // Main parsed "retry in 5 minutes; 2 seconds": the boundary window comes first.
    let joined = DiagnosticMessage::from("minutes; 2 seconds").with_prefix("retry in 5 ");
    assert_eq!(
        joined.reset_hint_at(observed()),
        Some(observed() + chrono::Duration::seconds(300))
    );
}

#[test]
fn r11_rendering_is_classified_as_86fbab995_stored_it() {
    // A bounded rendering adds "[truncated: 429 more chars]". 86fbab995 stored
    // that rendering in the error, so its classifiers read the marker too.
    let raw = "x".repeat(789);
    let bounded = DiagnosticMessage::from(raw.as_str())
        .map_text(|text| format!("{} [truncated: 429 more chars]", &text[..360]));
    let composed = LlmError::Http(bounded.with_prefix("peer: "));
    assert!(composed.mentions_status(429));
    assert!(bounded.with_prefix("peer: ").is_rate_limited());
}

#[test]
fn r8_redaction_after_a_cross_field_match_keeps_main_classification() {
    // Main joined "context window exceeded; <key>": the key follows the match.
    let fault = LlmError::Rejected {
        error_type: "context".into(),
        message: "window exceeded; sk-ant-abcdefghijklmnopqrstuvwxyz123456".into(),
    };
    assert!(!fault.to_string().contains("sk-ant-abcdef"));
    assert!(fault.is_context_window_exceeded());
    assert_eq!(
        fault.request_pressure_kind(),
        Some(RequestPressureKind::AggregateContext)
    );
}

#[test]
fn r9_credentials_never_invent_or_hide_reset_hints() {
    // Main parsed the raw text: a credential between "5" and "minutes" breaks
    // the duration, and provider text that looks like a redaction marker is
    // just provider text.
    for body in [
        "retry in 5 sk-ant-abcdefghijklmnopqrstuvwxyz123456 minutes",
        "retry in 5 ***REDACTED*** minutes",
    ] {
        let composed = DiagnosticMessage::from(body).with_prefix("peer: ");
        assert_eq!(composed.reset_hint_at(observed()), None, "{body}");
    }
    let beside =
        DiagnosticMessage::from("retry in 5 minutes; sk-ant-abcdefghijklmnopqrstuvwxyz123456")
            .with_prefix("peer: ");
    assert_eq!(
        beside.reset_hint_at(observed()),
        Some(observed() + chrono::Duration::seconds(300))
    );
}

// Credentials in every position the egress tests use: multi-line, adjacent.
const SECRETS: [&str; 4] = [
    "sk-ant-api03-EGRESSAAAAAAAAAAAAAAAAAAAAAA",
    "sk-ant-api03-EGRESSBBBBBBBBBBBBBBBBBBBBBB",
    "egress-bearer-token-ccc",
    "egress-cookie-value-ddd",
];
fn secret_body() -> String {
    format!(
        "rate limit 429\nAuthorization: Bearer {}\nCookie: session={}\n{}{} context window exceeded",
        SECRETS[2], SECRETS[3], SECRETS[0], SECRETS[1]
    )
}
fn assert_clean(surface: &str, text: &str) {
    for secret in SECRETS {
        assert!(!text.contains(secret), "{surface} leaked {secret}: {text}");
    }
}
fn every_variant() -> Vec<LlmError> {
    let body = secret_body();
    vec![
        LlmError::Http(body.as_str().into()),
        LlmError::Transport(body.as_str().into()),
        LlmError::Auth(body.as_str().into()),
        LlmError::Serialize(body.as_str().into()),
        LlmError::Unsupported(body.as_str().into()),
        LlmError::QuotaExceeded(body.as_str().into()),
        LlmError::RateLimited {
            message: body.as_str().into(),
            retry_after_secs: 1,
            from_provider: true,
            pipeline_retry_after_secs: None,
        },
        LlmError::Overloaded {
            message: body.as_str().into(),
            status: Some(503),
            retry_after_secs: None,
        },
        LlmError::Server {
            status: 400,
            message: body.as_str().into(),
            pipeline_retry_after_secs: None,
        },
        LlmError::Rejected {
            error_type: body.as_str().into(),
            message: body.as_str().into(),
        },
        LlmError::ProviderNotFound {
            name: body.as_str().into(),
            available: body.as_str().into(),
        },
        LlmError::ContextWindowExceeded {
            provider_message: body.as_str().into(),
            provider: Some(body.as_str().into()),
            model: Some(body.as_str().into()),
        },
    ]
}

#[test]
fn r9_egress_never_carries_raw_credentials() {
    for error in every_variant() {
        assert_clean("Display", &error.to_string());
        assert_clean("Debug", &format!("{error:?}"));
        assert_clean("Debug pretty", &format!("{error:#?}"));
        assert_clean("Serialize", &serde_json::to_string(&error).unwrap());
        let event = crate::provider_fault::error_event(error.clone());
        assert_clean("StreamEvent Debug", &format!("{event:?}"));
        assert_clean("StreamEvent JSON", &serde_json::to_string(&event).unwrap());
        // Error chains: std source walk and anyhow context in every format.
        let mut source: Option<&(dyn std::error::Error + 'static)> = Some(&error);
        while let Some(current) = source {
            assert_clean("source chain", &current.to_string());
            source = current.source();
        }
        let wrapped = anyhow::Error::new(error.clone()).context("provider call failed");
        assert_clean("anyhow Display", &format!("{wrapped}"));
        assert_clean("anyhow alternate", &format!("{wrapped:#}"));
        assert_clean("anyhow Debug", &format!("{wrapped:?}"));
        let message: String = match error {
            LlmError::Http(message) => message.into(),
            other => other.to_string(),
        };
        assert_clean("String conversion", &message);
    }
    let message = DiagnosticMessage::from(secret_body());
    assert_clean("Deref", &message);
    assert_clean("prefix", &message.with_prefix("peer: ").to_string());
    assert_clean(
        "map_text",
        &message.clone().map_text(str::to_string).to_string(),
    );
}

#[test]
fn r9_tracing_fields_never_carry_raw_credentials() {
    struct Writer(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);
    impl std::io::Write for Writer {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let output = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let captured = output.clone();
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_ansi(false)
        .with_writer(move || Writer(captured.clone()))
        .finish();
    // This callsite is unique to this test, so its cached interest is ours.
    tracing::subscriber::with_default(subscriber, || {
        for error in every_variant() {
            tracing::error!(display = %error, debug = ?error, "r9 egress probe");
        }
    });
    let text = String::from_utf8(output.lock().unwrap().clone()).unwrap();
    assert!(text.contains("r9 egress probe"), "capture empty: {text}");
    assert_clean("tracing", &text);
}
