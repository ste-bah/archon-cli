use super::*;

#[test]
fn segments_plain_english() {
    let t = "First sentence. Second one! And a third? Yes.";
    let s = segment_sentences(t);
    assert_eq!(s.len(), 4);
    assert_eq!(&t[s[0].0..s[0].1], "First sentence.");
    assert_eq!(&t[s[3].0..s[3].1], "Yes.");
}

#[test]
fn guards_abbreviations_initials_decimals() {
    let t = "Dr. King measured p = 0.0004 at 3.14 seconds. C. E. King agreed.";
    let s = segment_sentences(t);
    assert_eq!(
        s.len(),
        2,
        "{:?}",
        s.iter().map(|&(a, b)| &t[a..b]).collect::<Vec<_>>()
    );
}

#[test]
fn dotted_tokens_do_not_split() {
    let t = "We cite e.g. the U.S. FDA guidance. It applies.";
    let s = segment_sentences(t);
    assert_eq!(s.len(), 2);
}

#[test]
fn paragraph_break_terminates_without_punctuation() {
    let t = "A heading without punctuation\n\nThe body begins here.";
    let s = segment_sentences(t);
    assert_eq!(s.len(), 2);
    assert_eq!(&t[s[0].0..s[0].1], "A heading without punctuation");
}

#[test]
fn greek_question_mark_terminates() {
    let t = "τί ἐστιν ἡ ψυχή; ἡ ψυχὴ ἐντελέχεια σώματος.";
    let s = segment_sentences(t);
    assert_eq!(s.len(), 2);
    // byte spans slice cleanly on multibyte content
    for &(a, b) in &s {
        assert!(t.is_char_boundary(a) && t.is_char_boundary(b));
    }
}

#[test]
fn determinism() {
    let t = "One. Two. Three with e.g. a guard. Four?";
    assert_eq!(segment_sentences(t), segment_sentences(t));
}

#[test]
fn build_and_verify_roundtrip_in_mem() {
    use crate::schema::ensure_doc_schema;
    let db = DbInstance::new("mem", "", "").unwrap();
    ensure_doc_schema(&db).unwrap();
    let r = crate::ingest_text::ingest_text_source(
        &db,
        "corpus/test/sent.txt",
        "text/plain",
        "First sentence here. Second sentence follows! Greek: τί ἐστιν ἡ ψυχή; τέλος.",
    )
    .unwrap();
    let stats = rebuild_document(&db, &r.document_id).unwrap();
    assert!(stats.sentences >= 3, "sentences: {}", stats.sentences);
    let (checked, mismatches) = verify_sample(&db, 20, 42).unwrap();
    assert!(checked > 0);
    assert_eq!(mismatches, 0, "stored spans must re-slice byte-exact");
    // determinism: rebuild produces identical rows
    let s2 = rebuild_document(&db, &r.document_id).unwrap();
    assert_eq!(stats.sentences, s2.sentences);
}
