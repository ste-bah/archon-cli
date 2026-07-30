//! Table quality-gate + serialization (Port T — the portable, valuable half).
//!
//! Detection stays in Python (Marker `Table` blocks / PyMuPDF `find_tables` via the
//! sidecar) — Rust never reimplements it. What we port is the `is_real_table` GATE
//! (`table_extractor.py:162`, prose/TOC rejection) and tri-format serialization
//! (CSV / Markdown / JSON), faithful to `_rows_cols_to_table` / `_dataframe_to_*`.
//! Pure-Rust, deterministic, unit-tested.

use regex::Regex;

/// A detected table: page, cell grid (ragged allowed), bbox.
#[derive(Clone, Debug)]
pub struct TableGrid {
    pub page_num: u32,
    pub rows: Vec<Vec<String>>,
    pub bbox: [f32; 4],
}

/// Corpus-specific TOC/copyright/title-page markers (spec §3: make this a config list).
/// Defaults tuned for the dissertation corpus.
pub fn default_title_markers() -> Vec<String> {
    // Faithful to `table_extractor.py:235` — tuned for the dissertation corpus
    // (Princeton/Bollingen = the Aristotle Complete Works publisher).
    [
        "complete works",
        "revised oxford",
        "copyright",
        "isbn",
        "published by",
        "university press",
        "all rights reserved",
        "table of contents",
        "editors' introduction",
        "princeton university",
        "bollingen series",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

fn ncols(rows: &[Vec<String>]) -> usize {
    rows.iter().map(|r| r.len()).max().unwrap_or(0)
}

fn cell(rows: &[Vec<String>], r: usize, c: usize) -> &str {
    rows[r].get(c).map(|s| s.trim()).unwrap_or("")
}

/// Faithful port of `_is_real_table:162`. Rejects prose, phantom-column, TOC/title-page,
/// and 2-column-prose layouts; accepts genuine data tables.
pub fn is_real_table(rows: &[Vec<String>], title_markers: &[String]) -> bool {
    let nrows = rows.len();
    let cols = ncols(rows);
    if nrows < 3 || cols < 2 {
        return false;
    }
    let dlen = (nrows - 1) as f64; // data = rows[1..]
    let mut col_content = vec![0usize; cols];
    for r in 1..nrows {
        for (c, count) in col_content.iter_mut().enumerate() {
            if !cell(rows, r, c).is_empty() {
                *count += 1;
            }
        }
    }
    let meaningful = col_content
        .iter()
        .filter(|&&cc| cc as f64 >= 0.4 * dlen)
        .count();
    if meaningful < 2 {
        return false;
    }
    if cols > 3 && meaningful <= 2 {
        return false; // phantom whitespace columns
    }
    if nrows <= 4 && meaningful <= 3 && (col_content[0] as f64) < 0.3 * dlen {
        return false;
    }
    if rows[0].iter().all(|s| s.trim().is_empty()) {
        return false; // header row all-empty
    }
    // prose check over non-empty trimmed cells
    let cells: Vec<&str> = rows
        .iter()
        .flat_map(|r| r.iter().map(|s| s.trim()))
        .filter(|s| !s.is_empty())
        .collect();
    if cells.is_empty() {
        return false;
    }
    let avg_len =
        cells.iter().map(|s| s.chars().count()).sum::<usize>() as f64 / cells.len() as f64;
    if avg_len > 40.0 {
        return false; // prose, not tabular
    }
    if meaningful == 2 && nrows > 20 {
        return false; // 2-column prose layout
    }
    let long_frags = cells.iter().filter(|s| s.chars().count() > 80).count();
    if long_frags as f64 > 0.15 * cells.len() as f64 {
        return false;
    }
    let csv_lower = to_csv(rows).to_lowercase();
    if title_markers
        .iter()
        .any(|m| csv_lower.contains(&m.to_lowercase()))
    {
        return false; // TOC / copyright / title page
    }
    true
}

fn csv_field(s: &str) -> String {
    if s.contains(',') || s.contains('"') || s.contains('\n') {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

/// CSV, QUOTE_MINIMAL (faithful to the pandas default).
pub fn to_csv(rows: &[Vec<String>]) -> String {
    rows.iter()
        .map(|r| r.iter().map(|c| csv_field(c)).collect::<Vec<_>>().join(","))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Markdown pipe-table; pads short rows to `cols`, escapes `|`, first row = header.
pub fn to_markdown(rows: &[Vec<String>]) -> String {
    if rows.is_empty() {
        return String::new();
    }
    let cols = ncols(rows);
    let esc = |s: &str| s.replace('|', "\\|");
    let fmt = |r: &Vec<String>| -> String {
        let cells: Vec<String> = (0..cols)
            .map(|c| esc(r.get(c).map(|s| s.as_str()).unwrap_or("")))
            .collect();
        format!("| {} |", cells.join(" | "))
    };
    let mut out = vec![
        fmt(&rows[0]),
        format!("| {} |", vec!["---"; cols].join(" | ")),
    ];
    for r in &rows[1..] {
        out.push(fmt(r));
    }
    out.join("\n")
}

fn json_esc(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            '\r' => o.push_str("\\r"),
            '\t' => o.push_str("\\t"),
            c => o.push(c),
        }
    }
    o
}

/// JSON: list of row-dicts keyed by the header row.
pub fn to_json(rows: &[Vec<String>]) -> String {
    if rows.len() < 2 {
        return "[]".to_string();
    }
    let cols = ncols(rows);
    let header: Vec<String> = (0..cols)
        .map(|c| rows[0].get(c).cloned().unwrap_or_default())
        .collect();
    let objs: Vec<String> = rows[1..]
        .iter()
        .map(|r| {
            let pairs: Vec<String> = (0..cols)
                .map(|c| {
                    format!(
                        "\"{}\": \"{}\"",
                        json_esc(&header[c]),
                        json_esc(r.get(c).map(|s| s.as_str()).unwrap_or(""))
                    )
                })
                .collect();
            format!("{{{}}}", pairs.join(", "))
        })
        .collect();
    format!("[{}]", objs.join(", "))
}

/// `[TABLE] Page {p}, {r} rows × {c} columns\n` + context + markdown, as a Block.text
/// so it flows through the chunker like any block (`create_table_chunks:287`).
pub fn table_chunk_text(grid: &TableGrid, context_before: &str, context_after: &str) -> String {
    let r = grid.rows.len();
    let c = ncols(&grid.rows);
    let mut s = format!(
        "[TABLE] Page {}, {} rows × {} columns\n",
        grid.page_num, r, c
    );
    s.push_str(context_before);
    s.push_str(&to_markdown(&grid.rows));
    s.push_str(context_after);
    s
}

/// Parse a Marker/HTML `<table>` into a ragged cell grid: rows from `<tr>`, cells from
/// `<td>`/`<th>`, inner tags stripped (`crate::marker::strip_html`). Detection stays
/// upstream in Marker — this only structures an already-detected table's HTML so the
/// `is_real_table` gate and the serializers can run on it.
pub fn parse_table_html(html: &str) -> Vec<Vec<String>> {
    let row_re = Regex::new(r"(?is)<tr[^>]*>(.*?)</tr>").expect("static tr regex");
    let cell_re = Regex::new(r"(?is)<t[dh][^>]*>(.*?)</t[dh]>").expect("static cell regex");
    let mut rows = Vec::new();
    for rcap in row_re.captures_iter(html) {
        let mut cells = Vec::new();
        for ccap in cell_re.captures_iter(&rcap[1]) {
            cells.push(crate::marker::strip_html(&ccap[1]));
        }
        if !cells.is_empty() {
            rows.push(cells);
        }
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grid(rows: &[&[&str]]) -> Vec<Vec<String>> {
        rows.iter()
            .map(|r| r.iter().map(|s| s.to_string()).collect())
            .collect()
    }

    #[test]
    fn accepts_a_genuine_data_table() {
        let t = grid(&[
            &["Year", "Author", "Count"],
            &["2019", "Bogost", "12"],
            &["2020", "Calleja", "8"],
            &["2021", "Burke", "15"],
        ]);
        assert!(is_real_table(&t, &default_title_markers()));
    }

    #[test]
    fn rejects_too_small() {
        assert!(!is_real_table(&grid(&[&["a", "b"], &["1", "2"]]), &[])); // 2 rows
        assert!(!is_real_table(&grid(&[&["a"], &["1"], &["2"]]), &[])); // 1 col
    }

    #[test]
    fn rejects_prose_long_cells() {
        let t = grid(&[
            &["Heading one column", "Heading two column"],
            &[
                "This is a long sentence of prose that clearly is not tabular data at all.",
                "Another lengthy prose passage masquerading as a table cell here friend.",
            ],
            &[
                "Yet more flowing prose, the kind that PyMuPDF sometimes mistakes for a grid.",
                "And a final paragraph of prose to push the average cell length well past forty.",
            ],
        ]);
        assert!(
            !is_real_table(&t, &default_title_markers()),
            "avg cell length > 40 → prose"
        );
    }

    #[test]
    fn rejects_title_page_marker() {
        let t = grid(&[
            &["Aristotle", "x"],
            &["The Complete Works", "y"],
            &["Princeton University Press", "z"],
            &["Copyright 1984", "w"],
        ]);
        assert!(
            !is_real_table(&t, &default_title_markers()),
            "university press / copyright → reject"
        );
    }

    #[test]
    fn markdown_has_header_separator_and_escapes_pipes() {
        let md = to_markdown(&grid(&[&["a|b", "c"], &["1", "2"]]));
        assert!(md.contains("| a\\|b | c |"));
        assert!(md.contains("| --- | --- |"));
    }

    #[test]
    fn csv_quotes_minimally() {
        let csv = to_csv(&grid(&[&["a,b", "c"], &["plain", "x\"y"]]));
        assert_eq!(csv, "\"a,b\",c\nplain,\"x\"\"y\"");
    }

    #[test]
    fn json_keys_by_header() {
        let j = to_json(&grid(&[&["k1", "k2"], &["v1", "v2"]]));
        assert_eq!(j, "[{\"k1\": \"v1\", \"k2\": \"v2\"}]");
    }

    #[test]
    fn table_chunk_has_marker_header() {
        let g = TableGrid {
            page_num: 7,
            rows: grid(&[&["a", "b"], &["1", "2"]]),
            bbox: [0.0; 4],
        };
        let s = table_chunk_text(&g, "", "");
        assert!(s.starts_with("[TABLE] Page 7, 2 rows × 2 columns\n"));
    }

    #[test]
    fn parses_html_table_into_grid() {
        let html = "<table><tr><th>Year</th><th>Author</th></tr>\
                    <tr><td>2019</td><td><b>Bogost</b></td></tr>\
                    <tr><td>2020</td><td>Calleja</td></tr></table>";
        let g = parse_table_html(html);
        assert_eq!(g.len(), 3, "three rows");
        assert_eq!(g[0], vec!["Year".to_string(), "Author".to_string()]);
        assert_eq!(
            g[1],
            vec!["2019".to_string(), "Bogost".to_string()],
            "inner tags stripped"
        );
        assert!(is_real_table(&g, &default_title_markers()));
    }
}
