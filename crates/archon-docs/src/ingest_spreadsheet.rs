//! Spreadsheet extraction — renders workbooks (xlsx/xls) and CSV/TSV to
//! Markdown tables that flow through the existing text chunk pipeline.
//!
//! Each workbook sheet becomes its own section headed by the sheet name and
//! its own "page" in the extract result (page N == sheet N), so chunk page
//! anchors preserve tab identity. Sections are separated by a form feed
//! (matching the PDF page convention) wrapped in blank lines so the
//! paragraph chunker never merges two tabs' rows into one paragraph.

use std::io::Cursor;
use std::time::Instant;

use calamine::{Data, Range, Reader};

use crate::errors::DocsError;
use crate::models::PageOffset;
use crate::ocr::provider::OcrExtractResult;

/// Data rows per rendered table fragment. Large sheets are emitted as
/// multiple header-repeating tables separated by blank lines so the
/// paragraph chunker can split them; a monolithic table would land in a
/// single oversized chunk and get truncated at embedding time.
const ROWS_PER_TABLE_FRAGMENT: usize = 50;

/// Render spreadsheet bytes into the same shape the OCR providers produce,
/// so the rest of the ingest pipeline (pages → chunks → provenance → index)
/// runs unchanged.
pub(crate) fn extract_spreadsheet(
    media_type: &str,
    bytes: &[u8],
) -> Result<OcrExtractResult, DocsError> {
    let started = Instant::now();
    let (full_text, page_offsets) = match media_type {
        "text/csv" => render_delimited(bytes, ','),
        "text/tab-separated-values" => render_delimited(bytes, '\t'),
        _ => render_workbook(bytes)?,
    };
    Ok(OcrExtractResult {
        page_count: page_offsets.len() as u32,
        page_offsets,
        full_text,
        processing_duration_ms: started.elapsed().as_millis() as u64,
    })
}

fn render_workbook(bytes: &[u8]) -> Result<(String, Vec<PageOffset>), DocsError> {
    let mut workbook = calamine::open_workbook_auto_from_rs(Cursor::new(bytes)).map_err(|e| {
        DocsError::OcrApi {
            message: format!("failed to open spreadsheet workbook: {e}"),
            status_code: None,
        }
    })?;

    let sheet_names = workbook.sheet_names();
    if sheet_names.is_empty() {
        return Err(DocsError::OcrApi {
            message: "spreadsheet workbook contains no sheets".into(),
            status_code: None,
        });
    }

    let mut full_text = String::new();
    let mut offsets = Vec::with_capacity(sheet_names.len());
    for (i, name) in sheet_names.iter().enumerate() {
        let range = workbook
            .worksheet_range(name)
            .map_err(|e| DocsError::OcrApi {
                message: format!("failed to read sheet {name:?}: {e}"),
                status_code: None,
            })?;

        if i > 0 {
            full_text.push_str("\n\x0C\n\n");
        }
        let char_start = full_text.len();
        full_text.push_str("## ");
        full_text.push_str(name.trim());
        full_text.push_str("\n\n");

        let rows = trim_trailing_empty(range_to_rows(&range));
        if rows.is_empty() {
            full_text.push_str("*(empty sheet)*");
        } else {
            render_markdown_table(&mut full_text, &rows);
        }

        // Terminate non-final sections with a newline INSIDE the page range:
        // `page_for_offset` treats `char_end` as exclusive, so a paragraph
        // ending exactly at the boundary would otherwise anchor to the last
        // page instead of this sheet's page.
        if i + 1 < sheet_names.len() {
            full_text.push('\n');
        }
        offsets.push(PageOffset {
            page: (i + 1) as u32,
            char_start,
            char_end: full_text.len(),
        });
    }

    Ok((full_text, offsets))
}

/// Single-table render for CSV/TSV: one section, one page, no sheet heading.
fn render_delimited(bytes: &[u8], delimiter: char) -> (String, Vec<PageOffset>) {
    let text = String::from_utf8_lossy(bytes);
    let rows = trim_trailing_empty(parse_delimited(&text, delimiter));

    let mut full_text = String::new();
    if !rows.is_empty() {
        render_markdown_table(&mut full_text, &rows);
    }
    let offsets = vec![PageOffset {
        page: 1,
        char_start: 0,
        char_end: full_text.len(),
    }];
    (full_text, offsets)
}

fn range_to_rows(range: &Range<Data>) -> Vec<Vec<String>> {
    range
        .rows()
        .map(|row| row.iter().map(format_cell).collect())
        .collect()
}

/// Drop all-empty trailing rows, then all-empty trailing columns, and pad
/// ragged rows (CSV) so every row has the same column count.
fn trim_trailing_empty(mut rows: Vec<Vec<String>>) -> Vec<Vec<String>> {
    while rows
        .last()
        .is_some_and(|row| row.iter().all(|cell| cell.is_empty()))
    {
        rows.pop();
    }
    let width = rows
        .iter()
        .flat_map(|row| {
            row.iter()
                .enumerate()
                .filter(|(_, cell)| !cell.is_empty())
                .map(|(i, _)| i + 1)
        })
        .max()
        .unwrap_or(0);
    for row in &mut rows {
        row.truncate(width);
        row.resize(width, String::new());
    }
    rows
}

/// Emit `rows` as Markdown table fragments: first row is the header, data
/// rows are grouped into `ROWS_PER_TABLE_FRAGMENT`-sized tables that each
/// repeat the header, separated by blank lines.
fn render_markdown_table(out: &mut String, rows: &[Vec<String>]) {
    let header = &rows[0];
    let data = &rows[1..];

    let header_line = markdown_row(header);
    let separator_line = format!("|{}", " --- |".repeat(header.len()));

    let groups: Vec<&[Vec<String>]> = if data.is_empty() {
        vec![&[]]
    } else {
        data.chunks(ROWS_PER_TABLE_FRAGMENT).collect()
    };
    for (g, group) in groups.iter().enumerate() {
        if g > 0 {
            out.push_str("\n\n");
        }
        out.push_str(&header_line);
        out.push('\n');
        out.push_str(&separator_line);
        for row in *group {
            out.push('\n');
            out.push_str(&markdown_row(row));
        }
    }
}

fn markdown_row(cells: &[String]) -> String {
    let mut line = String::from("|");
    for cell in cells {
        line.push(' ');
        line.push_str(&escape_cell(cell));
        line.push_str(" |");
    }
    line
}

/// Keep cell text intact except for what would break table structure:
/// pipes are escaped and embedded line breaks become single spaces.
fn escape_cell(cell: &str) -> String {
    let mut out = String::with_capacity(cell.len());
    for ch in cell.trim().chars() {
        match ch {
            '|' => out.push_str("\\|"),
            '\r' => {}
            '\n' => out.push(' '),
            _ => out.push(ch),
        }
    }
    out
}

fn format_cell(cell: &Data) -> String {
    match cell {
        Data::Empty => String::new(),
        Data::String(s) => s.clone(),
        Data::Int(i) => i.to_string(),
        // f64 Display is the shortest round-trip form: 3.0 → "3", 2.5 → "2.5".
        Data::Float(f) => f.to_string(),
        Data::Bool(b) => if *b { "TRUE" } else { "FALSE" }.to_string(),
        Data::DateTime(dt) => format_excel_datetime(dt),
        Data::DateTimeIso(s) | Data::DurationIso(s) => s.clone(),
        Data::Error(e) => e.to_string(),
    }
}

fn format_excel_datetime(dt: &calamine::ExcelDateTime) -> String {
    if dt.is_datetime() {
        match dt.as_datetime() {
            Some(ndt) if ndt.time() == chrono::NaiveTime::MIN => ndt.format("%Y-%m-%d").to_string(),
            Some(ndt) => ndt.format("%Y-%m-%d %H:%M:%S").to_string(),
            None => dt.as_f64().to_string(),
        }
    } else {
        match dt.as_duration() {
            Some(d) => format!(
                "{:02}:{:02}:{:02}",
                d.num_hours(),
                d.num_minutes().abs() % 60,
                d.num_seconds().abs() % 60
            ),
            None => dt.as_f64().to_string(),
        }
    }
}

/// Minimal RFC 4180-style parser: quoted fields, `""` escapes, delimiter and
/// newlines allowed inside quotes. Enough for matching-standard exports
/// without pulling in a csv crate.
fn parse_delimited(text: &str, delimiter: char) -> Vec<Vec<String>> {
    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut row: Vec<String> = Vec::new();
    let mut field = String::new();
    let mut in_quotes = false;
    let mut chars = text.chars().peekable();

    while let Some(ch) = chars.next() {
        if in_quotes {
            if ch == '"' {
                if chars.peek() == Some(&'"') {
                    field.push('"');
                    chars.next();
                } else {
                    in_quotes = false;
                }
            } else {
                field.push(ch);
            }
        } else {
            match ch {
                '"' if field.is_empty() => in_quotes = true,
                '\r' => {}
                '\n' => {
                    row.push(std::mem::take(&mut field));
                    rows.push(std::mem::take(&mut row));
                }
                c if c == delimiter => row.push(std::mem::take(&mut field)),
                c => field.push(c),
            }
        }
    }
    if !field.is_empty() || !row.is_empty() {
        row.push(field);
        rows.push(row);
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn csv_renders_single_markdown_table() {
        let csv =
            b"scenario,list entry,expected\nS-1,ACME CORP,match\nS-2,\"Smith, John\",no match\n";
        let result = extract_spreadsheet("text/csv", csv).unwrap();

        assert_eq!(result.page_count, 1);
        assert_eq!(
            result.full_text,
            "| scenario | list entry | expected |\n\
             | --- | --- | --- |\n\
             | S-1 | ACME CORP | match |\n\
             | S-2 | Smith, John | no match |"
        );
    }

    #[test]
    fn tsv_splits_on_tabs() {
        let tsv = b"a\tb\n1\t2\n";
        let result = extract_spreadsheet("text/tab-separated-values", tsv).unwrap();
        assert!(result.full_text.starts_with("| a | b |"));
        assert!(result.full_text.contains("| 1 | 2 |"));
    }

    #[test]
    fn trailing_empty_rows_and_columns_are_dropped() {
        let csv = b"a,b,,\n1,2,,\n,,,\n,,,\n";
        let result = extract_spreadsheet("text/csv", csv).unwrap();
        assert_eq!(result.full_text, "| a | b |\n| --- | --- |\n| 1 | 2 |");
    }

    #[test]
    fn cell_text_with_pipes_and_newlines_stays_table_safe() {
        let csv = b"name,note\nx,\"has|pipe and\nnewline\"\n";
        let result = extract_spreadsheet("text/csv", csv).unwrap();
        assert!(result.full_text.contains("| has\\|pipe and newline |"));
    }

    #[test]
    fn empty_csv_produces_empty_text() {
        let result = extract_spreadsheet("text/csv", b"").unwrap();
        assert_eq!(result.full_text, "");
        assert_eq!(result.page_count, 1);
    }

    #[test]
    fn large_tables_split_into_header_repeating_fragments() {
        let mut csv = String::from("id,value\n");
        for i in 0..120 {
            csv.push_str(&format!("{i},v{i}\n"));
        }
        let result = extract_spreadsheet("text/csv", csv.as_bytes()).unwrap();
        // 120 data rows / 50 per fragment = 3 fragments, each led by the header.
        assert_eq!(result.full_text.matches("| id | value |").count(), 3);
        assert_eq!(result.full_text.matches("\n\n").count(), 2);
    }

    #[test]
    fn corrupt_workbook_is_an_extraction_error() {
        let result = extract_spreadsheet(
            "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
            b"not a zip archive",
        );
        assert!(matches!(result, Err(DocsError::OcrApi { .. })));
    }

    #[test]
    fn float_and_bool_cells_format_sensibly() {
        assert_eq!(format_cell(&Data::Float(3.0)), "3");
        assert_eq!(format_cell(&Data::Float(2.5)), "2.5");
        assert_eq!(format_cell(&Data::Int(-7)), "-7");
        assert_eq!(format_cell(&Data::Bool(true)), "TRUE");
        assert_eq!(format_cell(&Data::Empty), "");
    }

    #[test]
    fn excel_serial_dates_format_as_iso() {
        // 45292.0 == 2024-01-01 in the 1900 date system.
        let date =
            calamine::ExcelDateTime::new(45292.0, calamine::ExcelDateTimeType::DateTime, false);
        assert_eq!(format_cell(&Data::DateTime(date)), "2024-01-01");

        let datetime =
            calamine::ExcelDateTime::new(45292.5, calamine::ExcelDateTimeType::DateTime, false);
        assert_eq!(
            format_cell(&Data::DateTime(datetime)),
            "2024-01-01 12:00:00"
        );
    }
}
