//! Bounded receive-side HTTP evidence. No request body is retained.
use std::{collections::BTreeMap, pin::Pin, task::{Context, Poll}};
use archon_observability::transport::{EvidenceScope, current};
use futures_util::{Stream, StreamExt};
use serde_json::{Value, json};

pub(crate) struct Capture {
    scope: Option<EvidenceScope>,
    id: String,
    status: u16,
    headers: BTreeMap<String, String>,
    secrets: Vec<String>,
    first: Vec<u8>,
    last: Vec<u8>,
    line: Vec<u8>,
    skip_line: bool,
    bytes: usize,
    finish_reason: Option<String>,
    terminal: bool,
    end: &'static str,
}
impl Capture {
    pub fn new(response: &reqwest::Response, secrets: Vec<String>) -> Self {
        let mut capture = Self {
            scope: current(), id: uuid::Uuid::new_v4().to_string(),
            status: response.status().as_u16(), headers: BTreeMap::new(), secrets,
            first: vec![], last: vec![], line: vec![], skip_line: false, bytes: 0,
            finish_reason: None, terminal: false, end: "consumer_closed",
        };
        // Unknown headers can contain credentials. Keep their names, not values.
        for (name, value) in response.headers() {
            let value = if matches!(name.as_str(), "content-type" | "content-length" | "retry-after" | "x-request-id" | "request-id" | "date") {
                capture.scrub(value.to_str().unwrap_or("[non-text]"), false, false)
            } else { "[REDACTED]".into() };
            capture.headers.insert(name.to_string(), value);
        }
        capture.record("http_response_started");
        capture
    }
    pub fn push(&mut self, bytes: &[u8]) {
        if self.scope.is_none() { return; }
        self.bytes += bytes.len();
        self.first.extend_from_slice(&bytes[..bytes.len().min(500 - self.first.len())]);
        if bytes.len() >= 500 { self.last = bytes[bytes.len()-500..].to_vec(); }
        else {
            self.last.extend_from_slice(bytes);
            if self.last.len() > 500 { self.last.drain(..self.last.len()-500); }
        }
        // Only tiny metadata frames need decoding. A huge tool/text line is skipped,
        // not retained in memory merely to diagnose an empty result.
        for &byte in bytes {
            if byte == b'\n' {
                if !self.skip_line { self.decode_line(); }
                self.line.clear(); self.skip_line = false;
            } else if !self.skip_line {
                if self.line.len() < 65_536 { self.line.push(byte); }
                else { self.line.clear(); self.skip_line = true; }
            }
        }
    }
    fn decode_line(&mut self) {
        let line = String::from_utf8_lossy(&self.line);
        let text = line.trim().strip_prefix("data:").unwrap_or(line.trim()).trim();
        if text == "[DONE]" { self.terminal = true; return; }
        let Ok(value) = serde_json::from_str::<Value>(text) else { return; };
        if value["type"] == "message_stop" { self.terminal = true; }
        if let Some(reason) = value.pointer("/delta/stop_reason").or_else(|| value.pointer("/choices/0/finish_reason")).and_then(Value::as_str) {
            self.finish_reason = Some(self.scrub(reason, false, false));
        }
    }
    fn scrub(&self, text: &str, left_cut: bool, right_cut: bool) -> String {
        super::transport_evidence_redaction::scrub(text, &self.secrets, left_cut, right_cut)
    }
    fn record(&self, kind: &str) {
        if let Some(scope) = &self.scope {
            scope.record(json!({"kind":kind,"response_id":self.id,"http_status":self.status,
                "headers":self.headers,"body_bytes":self.bytes,"finish_reason":self.finish_reason,
                "terminal_marker":self.terminal,"stream_end":self.end,
                "body_first_500":self.scrub(&String::from_utf8_lossy(&self.first),false,self.bytes>500),
                "body_last_500":self.scrub(&String::from_utf8_lossy(&self.last),self.bytes>500,false)}));
        }
    }
    pub fn body(mut self, bytes: &[u8]) { self.push(bytes); self.end = "body_read"; }
}
impl Drop for Capture {
    fn drop(&mut self) { self.record("http_response"); }
}

pub(crate) fn stream(response: reqwest::Response, secrets: Vec<String>) -> impl Stream<Item = Result<impl AsRef<[u8]>, reqwest::Error>> + Send + Unpin + 'static {
    let capture = Capture::new(&response, secrets);
    Observed { inner: response.bytes_stream().boxed(), capture }
}
struct Observed<S> { inner: S, capture: Capture }
impl<S, B> Stream for Observed<S>
where S: Stream<Item = Result<B, reqwest::Error>> + Unpin, B: AsRef<[u8]> {
    type Item = Result<B, reqwest::Error>;
    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        match Pin::new(&mut this.inner).poll_next(cx) {
            Poll::Ready(Some(Ok(bytes))) => { this.capture.push(bytes.as_ref()); Poll::Ready(Some(Ok(bytes))) }
            Poll::Ready(Some(Err(error))) => { this.capture.end = "network_error"; Poll::Ready(Some(Err(error))) }
            Poll::Ready(None) => { this.capture.decode_line(); this.capture.end = "eof"; Poll::Ready(None) }
            Poll::Pending => Poll::Pending,
        }
    }
}
