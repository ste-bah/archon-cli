//! Terminal rendering tests for `archon docs answer`.
//!
//! These drive the sink and the tail renderer over in-memory writers, so they
//! assert on exactly the bytes an operator would see without needing a database
//! or a model.

use archon_pipeline::kb::query::QaSource;

use super::*;

/// A writer that counts flushes, so "the fragment was flushed" is testable.
///
/// A bare `Vec<u8>` cannot show this: its `flush` is a no-op, and an
/// unflushed terminal write is precisely the bug that would make streaming
/// look no different from waiting.
#[derive(Default)]
struct CountingWriter {
    bytes: Vec<u8>,
    flushes: usize,
}

impl Write for CountingWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.bytes.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.flushes += 1;
        Ok(())
    }
}

impl CountingWriter {
    fn text(&self) -> String {
        String::from_utf8(self.bytes.clone()).unwrap()
    }
}

fn result_with_one_source() -> QaQueryResult {
    QaQueryResult {
        answer: "Retention is thirty days.".into(),
        sources: vec![QaSource {
            chunk_id: "chunk-1".into(),
            document_id: "doc-1".into(),
            source_path: "policy.txt".into(),
            relevance_score: 0.75,
            quote: "Retention is thirty days.".into(),
        }],
        filed_document_id: None,
        search_duration_ms: 9,
        synthesis_duration_ms: 9_600,
        warnings: vec![],
    }
}

#[test]
fn every_fragment_is_written_in_order_and_flushed() {
    let mut sink = TerminalAnswerSink::new(
        CountingWriter::default(),
        CountingWriter::default(),
        Instant::now(),
    );

    for fragment in ["Retention ", "is ", "thirty days."] {
        sink.on_token(fragment).unwrap();
    }

    assert_eq!(sink.out.text(), "Retention is thirty days.");
    assert_eq!(sink.out.flushes, 3, "each fragment must reach the terminal");
    assert!(sink.wrote_body());
    assert!(sink.time_to_first_token_ms().is_some());
}

/// Retrieval notes qualify the answer, so they have to land above it.
#[test]
fn retrieval_warnings_are_printed_before_any_answer_text() {
    let mut sink = TerminalAnswerSink::new(
        CountingWriter::default(),
        CountingWriter::default(),
        Instant::now(),
    );

    sink.on_retrieved(&["no embedding provider".to_string()])
        .unwrap();

    assert!(sink.err.text().contains("Warning: no embedding provider"));
    assert!(
        sink.out.text().is_empty(),
        "the body must not have started yet"
    );

    sink.on_token("Retention is thirty days.").unwrap();
    assert_eq!(sink.out.text(), "Retention is thirty days.");
}

/// REQ-DOCS-014: the citations belong under the answer they support.
#[test]
fn citations_are_printed_after_the_streamed_body() {
    let mut out = CountingWriter::default();
    let mut err = CountingWriter::default();
    out.write_all(b"Retention is thirty days.").unwrap();

    render_tail(
        &mut out,
        &mut err,
        &result_with_one_source(),
        false,
        Some(420),
        9_600,
    )
    .unwrap();

    let text = out.text();
    let body = text.find("Retention is thirty days.").unwrap();
    let citations = text.find("Citations (1):").unwrap();
    assert!(body < citations, "{text}");
    assert!(
        text.contains("[1] chunk-1  score=0.750  policy.txt"),
        "{text}"
    );
    assert!(text.contains("first token 420ms, total 9600ms"), "{text}");
}

#[test]
fn an_answer_with_no_evidence_says_so_instead_of_listing_citations() {
    let mut out = CountingWriter::default();
    let mut err = CountingWriter::default();
    let result = QaQueryResult {
        sources: vec![],
        ..result_with_one_source()
    };

    render_tail(&mut out, &mut err, &result, false, Some(12), 30).unwrap();

    assert!(out.text().contains("No supporting evidence was found"));
    assert!(!out.text().contains("Citations"));
}

/// The budget is about how long the operator waits, and a streamed answer ends
/// that wait at the first character.
#[test]
fn a_fast_first_token_does_not_warn_even_when_the_total_is_over_budget() {
    let mut out = CountingWriter::default();
    let mut err = CountingWriter::default();

    render_tail(
        &mut out,
        &mut err,
        &result_with_one_source(),
        false,
        Some(420),
        9_600,
    )
    .unwrap();

    assert!(!err.text().contains("budget"), "{}", err.text());
}

#[test]
fn a_slow_first_token_warns() {
    let mut out = CountingWriter::default();
    let mut err = CountingWriter::default();

    render_tail(
        &mut out,
        &mut err,
        &result_with_one_source(),
        false,
        Some(6_100),
        9_600,
    )
    .unwrap();

    assert!(
        err.text()
            .contains("Warning: the first token took 6100ms, over the 5000ms budget."),
        "{}",
        err.text()
    );
}

/// Nothing streamed (an empty model response): the budget falls back to the
/// total, because that is then the whole wait.
#[test]
fn an_unstreamed_answer_is_judged_on_its_total_and_reports_synthesis_time() {
    let mut out = CountingWriter::default();
    let mut err = CountingWriter::default();

    render_tail(
        &mut out,
        &mut err,
        &result_with_one_source(),
        false,
        None,
        9_600,
    )
    .unwrap();

    assert!(
        out.text().contains("Retrieval 9ms, synthesis 9600ms"),
        "{}",
        out.text()
    );
    assert!(
        err.text()
            .contains("Warning: the answer took 9600ms, over the 5000ms budget.")
    );
}

#[test]
fn a_filed_answer_reports_its_document_id_and_an_unfiled_one_warns() {
    let mut out = CountingWriter::default();
    let mut err = CountingWriter::default();
    let filed = QaQueryResult {
        filed_document_id: Some("doc-answer-1".into()),
        ..result_with_one_source()
    };
    render_tail(&mut out, &mut err, &filed, true, Some(10), 20).unwrap();
    assert!(out.text().contains("Filed as document doc-answer-1"));

    let mut out = CountingWriter::default();
    let mut err = CountingWriter::default();
    render_tail(
        &mut out,
        &mut err,
        &result_with_one_source(),
        true,
        Some(10),
        20,
    )
    .unwrap();
    assert!(err.text().contains("the answer could not be filed"));
}
