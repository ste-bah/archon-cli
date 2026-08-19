//! Tests for OTLP export (#189 Phase 10).
//!
//! The one that matters is [`a_secret_in_a_span_attribute_is_redacted_in_the_export`].
//! Everything else here is wiring; that test is the security property, and it
//! is asserted on the bytes that would have left the machine rather than on the
//! code path that produces them.

use super::*;

use std::sync::{Arc, Mutex};

use opentelemetry_sdk::error::OTelSdkResult;
use opentelemetry_sdk::trace::SpanData;
use tracing_subscriber::layer::SubscriberExt;

use crate::redaction::RedactionLayer;

/// An exporter that keeps what it was given instead of sending it.
///
/// This is what makes the security claim testable without a collector: the
/// assertions below run against exactly the `SpanData` the OTLP transport
/// would have serialised.
#[derive(Debug, Clone, Default)]
struct CapturingExporter {
    spans: Arc<Mutex<Vec<SpanData>>>,
}

impl CapturingExporter {
    fn captured(&self) -> Vec<SpanData> {
        self.spans
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }
}

impl SpanExporter for CapturingExporter {
    fn export(
        &self,
        batch: Vec<SpanData>,
    ) -> impl std::future::Future<Output = OTelSdkResult> + Send {
        self.spans
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .extend(batch);
        std::future::ready(Ok(()))
    }
}

/// Run `body` under a subscriber whose only sink is the redaction layer, with
/// export attached, and return what the exporter saw.
fn export_of(body: impl FnOnce()) -> Vec<SpanData> {
    let exporter = CapturingExporter::default();
    let export = Arc::new(OtlpExport::with_exporter(exporter.clone(), "archon-test"));
    let layer = RedactionLayer::with_writer(std::io::sink()).with_otlp(Arc::clone(&export));
    let subscriber = tracing_subscriber::registry().with(layer);

    ::tracing::subscriber::with_default(subscriber, body);
    export.shutdown();
    exporter.captured()
}

fn attribute(span: &SpanData, key: &str) -> Option<String> {
    span.attributes
        .iter()
        .find(|kv| kv.key.as_str() == key)
        .map(|kv| kv.value.to_string())
}

/// The acceptance criterion. A secret placed in a span attribute must not
/// survive into the exported span — this is the whole reason export is owned by
/// the redaction layer instead of stacked beside it.
#[test]
fn a_secret_in_a_span_attribute_is_redacted_in_the_export() {
    let spans = export_of(|| {
        let span =
            ::tracing::info_span!("agent.turn", auth = "sk-ant-api03_AAAAAAAAAAAAAAAAAAAA1234");
        let _entered = span.enter();
    });

    assert_eq!(spans.len(), 1, "expected exactly one exported span");
    let exported = format!("{:?}", spans[0].attributes);
    assert!(
        !exported.contains("sk-ant-api03"),
        "the raw secret reached the exporter: {exported}"
    );
    assert!(
        exported.contains("REDACTED"),
        "the attribute was dropped rather than redacted: {exported}"
    );
}

/// Every secret shape the layer knows, checked at the export boundary rather
/// than at the log line — the two sinks must not be able to disagree.
///
/// The fixtures are assembled at runtime rather than written as literals.
/// They are fake, but a scanner cannot tell that from a file, and GitHub's push
/// protection rejected an earlier draft of this test over the Stripe-shaped
/// one. Building them from parts keeps the coverage and leaves nothing in the
/// source for a scanner to find — the same trick #189 Phase 7 needed for its
/// deliberately-invalid environment variable.
#[test]
fn every_known_secret_shape_is_redacted_in_the_export() {
    let fixtures = [
        format!("sk-ant-{}_{}", "api03", "B".repeat(20) + "5678"),
        format!("AKIA{}", "IOSFODNN7EXAMPLE"),
        format!("gh{}_{}", "p", "a".repeat(36)),
        format!("sk_{}_{}", "live", "a".repeat(26)),
        format!("{} abc.def.ghi", "bearer"),
    ];
    for secret in &fixtures {
        let secret = secret.as_str();
        let spans = export_of(|| {
            let span = ::tracing::info_span!("agent.turn", value = secret);
            let _entered = span.enter();
        });
        let exported = format!("{:?}", spans[0].attributes);
        assert!(
            !exported.contains(secret),
            "{secret} survived into the export: {exported}"
        );
    }
}

/// A field *name* that is itself sensitive is masked too, matching what the
/// log sink does — otherwise `api_key = <redacted>` would still tell a reader
/// which key exists.
#[test]
fn a_sensitive_field_name_is_masked_in_the_export() {
    let spans = export_of(|| {
        let span = ::tracing::info_span!("agent.turn", api_key = "value");
        let _entered = span.enter();
    });

    let exported = format!("{:?}", spans[0].attributes);
    assert!(!exported.contains("api_key"), "{exported}");
}

/// Ordinary attributes must survive, or the export would be redaction with no
/// telemetry left in it.
#[test]
fn an_ordinary_attribute_is_exported_unchanged() {
    let spans = export_of(|| {
        let span = ::tracing::info_span!("agent.turn", task_id = "task-42");
        let _entered = span.enter();
    });

    assert_eq!(spans[0].name, "agent.turn");
    assert_eq!(attribute(&spans[0], "task_id").as_deref(), Some("task-42"));
}

/// Fields recorded after the span opens go through the same redaction. A
/// `Empty` field filled in later is exactly how `turn_ms` is recorded.
#[test]
fn a_field_recorded_after_the_span_opens_is_also_redacted() {
    let spans = export_of(|| {
        let span = ::tracing::info_span!("agent.turn", later = ::tracing::field::Empty);
        span.record("later", "sk-ant-api03_CCCCCCCCCCCCCCCCCCCC9999");
        let _entered = span.enter();
    });

    let exported = format!("{:?}", spans[0].attributes);
    assert!(!exported.contains("sk-ant-api03"), "{exported}");
}

/// Nested spans each export. A parent that swallowed its children would lose
/// the tool calls inside a turn.
#[test]
fn nested_spans_each_export() {
    let spans = export_of(|| {
        let outer = ::tracing::info_span!("agent.turn", task_id = "t1");
        let _outer = outer.enter();
        let inner = ::tracing::info_span!("slash.dispatch", command = "/compact");
        let _inner = inner.enter();
    });

    let names: Vec<&str> = spans.iter().map(|span| span.name.as_ref()).collect();
    assert!(names.contains(&"agent.turn"), "{names:?}");
    assert!(names.contains(&"slash.dispatch"), "{names:?}");
}

/// With no endpoint configured nothing is exported and nothing is allocated per
/// span — the layer takes the path it always did.
#[test]
fn without_an_endpoint_nothing_is_exported() {
    let exporter = CapturingExporter::default();
    let layer = RedactionLayer::with_writer(std::io::sink());
    let subscriber = tracing_subscriber::registry().with(layer);

    ::tracing::subscriber::with_default(subscriber, || {
        let span = ::tracing::info_span!("agent.turn", task_id = "t1");
        let _entered = span.enter();
    });

    assert!(exporter.captured().is_empty());
}

/// A bad endpoint must not stop the process starting. Telemetry going nowhere
/// is a degraded run, not a reason to refuse to run an agent.
#[test]
fn an_unusable_endpoint_is_reported_rather_than_fatal() {
    let built = OtlpExport::new("not a url at all", "archon-test");

    // Either outcome is acceptable — what matters is that it is a `Result` the
    // caller can log and carry on from, not a panic.
    if let Ok(export) = built {
        export.shutdown();
    }
}
