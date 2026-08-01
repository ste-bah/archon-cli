//! Spreadsheet ingest tests — multi-tab xlsx, CSV, dedupe, and
//! unsupported-type regressions, mirroring `ingest_tests.rs`.

use super::test_support::*;
use super::*;

/// Build a minimal but valid .xlsx in memory: one worksheet part per
/// `(sheet_name, rows)` entry, inline strings for text and `<v>` values for
/// cells that parse as numbers. Stored (uncompressed) zip entries keep the
/// fixture dependency-light.
fn build_xlsx(sheets: &[(&str, &[&[&str]])]) -> Vec<u8> {
    use std::io::Write as _;

    let options =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));

    let mut content_types = String::from(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
<Default Extension="xml" ContentType="application/xml"/>
<Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/>"#,
    );
    let mut workbook_sheets = String::new();
    let mut workbook_rels = String::new();
    for (i, (name, _)) in sheets.iter().enumerate() {
        let n = i + 1;
        content_types.push_str(&format!(
            r#"<Override PartName="/xl/worksheets/sheet{n}.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>"#
        ));
        workbook_sheets.push_str(&format!(
            r#"<sheet name="{name}" sheetId="{n}" r:id="rId{n}"/>"#
        ));
        workbook_rels.push_str(&format!(
            r#"<Relationship Id="rId{n}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet{n}.xml"/>"#
        ));
    }
    content_types.push_str("</Types>");

    writer.start_file("[Content_Types].xml", options).unwrap();
    writer.write_all(content_types.as_bytes()).unwrap();

    writer.start_file("_rels/.rels", options).unwrap();
    writer
        .write_all(
            br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/>
</Relationships>"#,
        )
        .unwrap();

    writer.start_file("xl/workbook.xml", options).unwrap();
    writer
        .write_all(
            format!(
                r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
<sheets>{workbook_sheets}</sheets>
</workbook>"#
            )
            .as_bytes(),
        )
        .unwrap();

    writer
        .start_file("xl/_rels/workbook.xml.rels", options)
        .unwrap();
    writer
        .write_all(
            format!(
                r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">{workbook_rels}</Relationships>"#
            )
            .as_bytes(),
        )
        .unwrap();

    for (i, (_, rows)) in sheets.iter().enumerate() {
        let mut sheet_xml = String::from(
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData>"#,
        );
        for (r, row) in rows.iter().enumerate() {
            sheet_xml.push_str(&format!("<row r=\"{}\">", r + 1));
            for (c, cell) in row.iter().enumerate() {
                let col = char::from(b'A' + c as u8);
                if cell.parse::<f64>().is_ok() {
                    sheet_xml.push_str(&format!("<c r=\"{col}{}\"><v>{cell}</v></c>", r + 1));
                } else {
                    sheet_xml.push_str(&format!(
                        "<c r=\"{col}{}\" t=\"inlineStr\"><is><t>{cell}</t></is></c>",
                        r + 1
                    ));
                }
            }
            sheet_xml.push_str("</row>");
        }
        sheet_xml.push_str("</sheetData></worksheet>");

        writer
            .start_file(format!("xl/worksheets/sheet{}.xml", i + 1), options)
            .unwrap();
        writer.write_all(sheet_xml.as_bytes()).unwrap();
    }

    writer.finish().unwrap().into_inner()
}

/// Matching-standard style fixture: a scenarios tab and a list-entries tab,
/// each with enough rows that the chunker flushes one chunk per tab.
fn matching_standard_xlsx() -> Vec<u8> {
    let scenarios: &[&[&str]] = &[
        &["scenario", "list entry", "transaction", "expected"],
        &[
            "S-01 exact name",
            "ACME GLOBAL TRADING LLC",
            "wire to ACME GLOBAL TRADING",
            "match",
        ],
        &[
            "S-02 transliteration",
            "OOO VOSTOK ENERGO",
            "payment to VOSTOK ENERGO LLC",
            "match",
        ],
        &[
            "S-03 unrelated party",
            "NORTHWIND CARGO SA",
            "invoice from SOUTHSEA FOODS",
            "no match",
        ],
    ];
    let list_entries: &[&[&str]] = &[
        &["entry id", "name", "program", "score threshold"],
        &["E-1001", "ACME GLOBAL TRADING LLC", "SDN", "0.92"],
        &["E-1002", "OOO VOSTOK ENERGO", "SSI", "0.9"],
        &["E-1003", "NORTHWIND CARGO SA", "SDN", "0.88"],
    ];
    build_xlsx(&[("Scenarios", scenarios), ("List Entries", list_entries)])
}

#[tokio::test]
async fn test_ingest_multi_tab_xlsx_keeps_tab_sections_and_page_anchors() {
    let db = test_db();
    let dir = tempfile::tempdir().unwrap();
    let file_path = dir.path().join("matching-standard.xlsx");
    fs::write(&file_path, matching_standard_xlsx()).unwrap();

    let r = ingest_file(&db, &file_path).await.unwrap();
    assert!(r.was_new);
    assert!(!r.ocr_skipped, "spreadsheets run the extract pipeline");
    assert!(!r.pipeline_failed);
    let doc_id = r.document_id;

    let doc = store::get_doc_source(&db, &doc_id).unwrap().unwrap();
    assert_eq!(
        doc.media_type,
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"
    );
    assert_eq!(doc.status, DocumentStatus::Ingested);

    // One page per sheet, in tab order.
    let pages = store::list_pages_for_doc(&db, &doc_id).unwrap();
    assert_eq!(pages.len(), 2, "one page per sheet");

    let ocr_runs = store::list_ocr_runs_for_doc(&db, &doc_id).unwrap();
    assert_eq!(ocr_runs.len(), 1);
    assert_eq!(ocr_runs[0].status, OcrStatus::Completed);

    // Every tab is a Markdown section headed by its sheet name, and cell
    // text survives the render.
    let chunks = store::list_chunks_for_doc(&db, &doc_id).unwrap();
    let all_text: String = chunks.iter().map(|c| c.content.as_str()).collect();
    assert!(all_text.contains("## Scenarios"));
    assert!(all_text.contains("## List Entries"));
    assert!(all_text.contains("| S-02 transliteration |"));
    assert!(all_text.contains("| E-1003 | NORTHWIND CARGO SA | SDN | 0.88 |"));

    // Tab context survives chunking: the chunk carrying the second tab's
    // rows is anchored to page 2, and no chunk mixes rows from both tabs.
    let list_chunk = chunks
        .iter()
        .find(|c| c.content.contains("E-1001"))
        .expect("list-entries rows must be chunked");
    assert!(list_chunk.content.contains("## List Entries"));
    assert_eq!(list_chunk.page_start, 2);
    assert!(
        !list_chunk.content.contains("S-01"),
        "chunks must not mix rows across tabs"
    );
    let scenario_chunk = chunks
        .iter()
        .find(|c| c.content.contains("S-01"))
        .expect("scenarios rows must be chunked");
    assert_eq!(scenario_chunk.page_start, 1);
    assert_eq!(scenario_chunk.page_end, 1);
}

#[tokio::test]
async fn test_ingest_csv_file() {
    let db = test_db();
    let dir = tempfile::tempdir().unwrap();
    let file_path = dir.path().join("transactions.csv");
    fs::write(
        &file_path,
        "scenario,transaction,expected\n\
         S-1,wire to ACME GLOBAL,match\n\
         S-2,\"payment, split tranche\",no match\n",
    )
    .unwrap();

    let r = ingest_file(&db, &file_path).await.unwrap();
    assert!(r.was_new);
    assert!(!r.pipeline_failed);

    let doc = store::get_doc_source(&db, &r.document_id).unwrap().unwrap();
    assert_eq!(doc.media_type, "text/csv");
    assert_eq!(doc.status, DocumentStatus::Ingested);

    let chunks = store::list_chunks_for_doc(&db, &r.document_id).unwrap();
    assert!(!chunks.is_empty(), "expected at least one chunk");
    assert!(
        chunks[0]
            .content
            .contains("| scenario | transaction | expected |")
    );
    assert!(
        chunks[0]
            .content
            .contains("| S-2 | payment, split tranche | no match |")
    );
}

#[tokio::test]
async fn test_ingest_spreadsheet_deduplicates_by_content_hash() {
    let db = test_db();
    let dir = tempfile::tempdir().unwrap();
    let bytes = matching_standard_xlsx();
    fs::write(dir.path().join("first.xlsx"), &bytes).unwrap();
    fs::write(dir.path().join("second.xlsx"), &bytes).unwrap();

    let result = ingest_directory(&db, dir.path()).await.unwrap();
    assert_eq!(result.sources_registered, 1);
    assert_eq!(result.sources_skipped_duplicate, 1);
    assert_eq!(result.sources_failed, 0);

    let sources = store::list_doc_sources(&db).unwrap();
    assert_eq!(sources.len(), 1);
}

#[tokio::test]
async fn test_ingest_directory_discovers_spreadsheets_and_skips_binaries() {
    let db = test_db();
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("standard.xlsx"), matching_standard_xlsx()).unwrap();
    fs::write(dir.path().join("cases.csv"), "a,b\n1,2\n").unwrap();
    fs::write(dir.path().join("skip.bin"), b"binary").unwrap();
    fs::write(dir.path().join("skip.ods"), b"not mapped yet").unwrap();

    let result = ingest_directory(&db, dir.path()).await.unwrap();
    assert_eq!(result.sources_registered, 2, "xlsx + csv only");
    assert_eq!(result.sources_failed, 0);

    let sources = store::list_doc_sources(&db).unwrap();
    assert_eq!(sources.len(), 2);
}

#[tokio::test]
async fn test_ingest_unsupported_types_still_rejected() {
    let db = test_db();
    let dir = tempfile::tempdir().unwrap();
    for name in ["data.ods", "data.parquet", "data.numbers"] {
        let file_path = dir.path().join(name);
        fs::write(&file_path, b"spreadsheet-adjacent binary").unwrap();
        let result = ingest_file(&db, &file_path).await;
        match result {
            Err(DocsError::UnsupportedMediaType { media_type }) => {
                assert_eq!(media_type, "application/octet-stream");
            }
            other => panic!("{name}: expected UnsupportedMediaType, got {other:?}"),
        }
    }
}

#[tokio::test]
async fn test_ingest_corrupt_xlsx_marks_document_failed() {
    let db = test_db();
    let dir = tempfile::tempdir().unwrap();
    let file_path = dir.path().join("broken.xlsx");
    fs::write(&file_path, b"this is not a zip archive").unwrap();

    let r = ingest_file(&db, &file_path).await.unwrap();
    assert!(
        r.was_new,
        "document is registered even when extraction fails"
    );
    assert!(r.pipeline_failed);

    let doc = store::get_doc_source(&db, &r.document_id).unwrap().unwrap();
    assert_eq!(doc.status, DocumentStatus::Failed);

    let ocr_runs = store::list_ocr_runs_for_doc(&db, &r.document_id).unwrap();
    assert_eq!(ocr_runs.len(), 1);
    assert_eq!(ocr_runs[0].status, OcrStatus::Failed);
}
