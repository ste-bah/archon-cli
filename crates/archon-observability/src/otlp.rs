//! OTLP span export, driven by the redaction layer (#189 Phase 10).
//!
//! # Why this is not a `tracing` layer
//!
//! The obvious implementation is `tracing-opentelemetry` stacked on the
//! registry next to [`crate::redaction::RedactionLayer`]. It would be a
//! catastrophic secret leak, for the reason the redaction module's tombstone
//! comment already records: **tracing layers are parallel sinks, not a
//! pipeline**. Both layers would see the same raw event, and only one of them
//! redacts. The other would ship the unscrubbed copy off the machine.
//!
//! So the export is not a layer. `RedactionLayer` owns it, and hands it values
//! it has already scrubbed. There is exactly one path from an event to the
//! wire, and redaction is on it.
//!
//! Everything here is inert unless `[observability] otlp_endpoint` is set.

use std::borrow::Cow;
use std::time::SystemTime;

use opentelemetry::trace::{Span as _, SpanBuilder, Tracer, TracerProvider};
use opentelemetry::{KeyValue, global};
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::Resource;
use opentelemetry_sdk::trace::{SdkTracerProvider, SpanExporter};

/// Instrumentation scope name on every exported span.
const SCOPE: &str = "archon";

/// One redacted span on its way out.
///
/// Built by the redaction layer from values it has already scrubbed, so
/// nothing here needs to be trusted or re-checked — and nothing else in the
/// process can reach the exporter to bypass that.
pub(crate) struct RedactedSpan {
    pub name: String,
    pub attributes: Vec<(String, String)>,
    pub start: SystemTime,
    pub end: SystemTime,
}

/// A configured export destination.
pub struct OtlpExport {
    provider: SdkTracerProvider,
}

impl std::fmt::Debug for OtlpExport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("OtlpExport")
    }
}

impl OtlpExport {
    /// Build an exporter pointed at `endpoint`, e.g.
    /// `http://127.0.0.1:4318/v1/traces`.
    ///
    /// HTTP/protobuf rather than the default gRPC transport: it rides the
    /// `reqwest` this workspace already builds, where gRPC would add tonic and
    /// prost for a transport nothing else here needs.
    pub fn new(endpoint: &str, service_name: &str) -> anyhow::Result<Self> {
        let exporter = opentelemetry_otlp::SpanExporter::builder()
            .with_http()
            .with_endpoint(endpoint)
            .build()
            .map_err(|error| anyhow::anyhow!("OTLP exporter for {endpoint}: {error}"))?;
        Ok(Self::with_exporter(exporter, service_name))
    }

    /// As [`Self::new`], but against a caller-supplied exporter.
    ///
    /// This is what makes the security property testable: a test can supply a
    /// capturing exporter and assert on the bytes that would have left the
    /// machine, with no collector and no network.
    pub(crate) fn with_exporter<E: SpanExporter + 'static>(
        exporter: E,
        service_name: &str,
    ) -> Self {
        Self {
            provider: SdkTracerProvider::builder()
                // Simple rather than batch: a batch processor needs a runtime
                // to drain on, and this has to work from `init_tracing`, which
                // runs before one exists and is also called from tests.
                .with_simple_exporter(exporter)
                .with_resource(
                    Resource::builder()
                        .with_attribute(KeyValue::new("service.name", service_name.to_string()))
                        .build(),
                )
                .build(),
        }
    }

    /// Export one already-redacted span.
    pub(crate) fn export(&self, span: RedactedSpan) {
        let tracer = self.provider.tracer(SCOPE);
        let attributes: Vec<KeyValue> = span
            .attributes
            .into_iter()
            .map(|(key, value)| KeyValue::new(Cow::Owned(key), value))
            .collect();
        let builder = SpanBuilder::from_name(span.name)
            .with_start_time(span.start)
            .with_attributes(attributes);
        // Both timestamps come from the recorded span, not from now: building
        // and ending it live would time the export instead of the work.
        let mut exported = tracer.build(builder);
        exported.end_with_timestamp(span.end);
    }

    /// Flush anything still queued. Called on shutdown.
    pub fn shutdown(&self) {
        if let Err(error) = self.provider.shutdown() {
            ::tracing::debug!(%error, "otlp: shutdown reported an error");
        }
    }
}

/// Install `provider` as the global one, so anything reaching for
/// `global::tracer` sees the same destination rather than a silent no-op.
pub(crate) fn set_global(provider: &OtlpExport) {
    global::set_tracer_provider(provider.provider.clone());
}

#[cfg(test)]
#[path = "otlp_tests.rs"]
mod tests;
