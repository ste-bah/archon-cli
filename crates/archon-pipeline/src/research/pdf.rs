use std::fs;
use std::path::Path;

use anyhow::Result;

use super::final_artifact::{Appendix, ResearchPaper};

const PAGE_W: f32 = 612.0;
const PAGE_H: f32 = 792.0;
const MARGIN: f32 = 72.0;
const FONT_SIZE: f32 = 12.0;
const LINE_H: f32 = 24.0;
const BODY_W: f32 = PAGE_W - (MARGIN * 2.0);
const INDENT: f32 = 36.0;

pub fn write_research_pdf(path: &Path, paper: &ResearchPaper) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut doc = PdfLayout::new();
    doc.title_page(&paper.title);
    doc.abstract_page(&paper.abstract_text);
    doc.body_pages(paper);
    fs::write(path, render_pdf(&doc.pages))?;
    Ok(())
}

struct PdfLayout {
    pages: Vec<Vec<TextLine>>,
    y: f32,
}

#[derive(Clone)]
struct TextLine {
    x: f32,
    y: f32,
    font: &'static str,
    size: f32,
    text: String,
}

impl PdfLayout {
    fn new() -> Self {
        Self {
            pages: Vec::new(),
            y: PAGE_H - MARGIN,
        }
    }

    fn title_page(&mut self, title: &str) {
        self.new_page();
        self.y = PAGE_H - 260.0;
        self.center(title, "F2", 12.0);
        self.y -= LINE_H * 2.0;
        self.center("Archon Research Pipeline", "F1", 12.0);
    }

    fn abstract_page(&mut self, abstract_text: &str) {
        self.new_page();
        self.center("Abstract", "F2", 12.0);
        self.y -= LINE_H;
        for paragraph in paragraphs(abstract_text) {
            self.paragraph(&paragraph, 0.0, 0.0);
        }
    }

    fn body_pages(&mut self, paper: &ResearchPaper) {
        self.new_page();
        self.center(&paper.title, "F2", 12.0);
        self.y -= LINE_H;
        self.markdown_body(&paper.body_markdown);
        self.new_page();
        self.center("References", "F2", 12.0);
        self.y -= LINE_H;
        for entry in &paper.references {
            self.paragraph(entry, 0.0, INDENT);
        }
        for appendix in &paper.appendices {
            self.appendix(appendix);
        }
    }

    fn appendix(&mut self, appendix: &Appendix) {
        self.new_page();
        self.center(&appendix.title, "F2", 12.0);
        self.y -= LINE_H;
        self.markdown_body(&appendix.body);
    }

    fn markdown_body(&mut self, markdown: &str) {
        let mut paragraph = String::new();
        for line in markdown.lines() {
            if let Some(title) = heading_title(line) {
                self.flush_paragraph(&mut paragraph);
                self.y -= LINE_H / 2.0;
                self.left(&title, "F2", 12.0, MARGIN);
                self.y -= LINE_H / 2.0;
            } else if line.trim().is_empty() {
                self.flush_paragraph(&mut paragraph);
            } else {
                if !paragraph.is_empty() {
                    paragraph.push(' ');
                }
                paragraph.push_str(line.trim());
            }
        }
        self.flush_paragraph(&mut paragraph);
    }

    fn flush_paragraph(&mut self, paragraph: &mut String) {
        if !paragraph.trim().is_empty() {
            self.paragraph(paragraph.trim(), INDENT, 0.0);
            paragraph.clear();
        }
    }

    fn paragraph(&mut self, text: &str, first_indent: f32, hanging_indent: f32) {
        let lines = wrap_text(text, BODY_W - first_indent.max(hanging_indent), FONT_SIZE);
        for (idx, line) in lines.iter().enumerate() {
            let x = if hanging_indent > 0.0 && idx > 0 {
                MARGIN + hanging_indent
            } else {
                MARGIN + first_indent
            };
            self.left(line, "F1", FONT_SIZE, x);
        }
        self.y -= LINE_H / 2.0;
    }

    fn center(&mut self, text: &str, font: &'static str, size: f32) {
        for line in wrap_text(text, BODY_W, size) {
            let approx_w = line.chars().count() as f32 * size * 0.5;
            let x = ((PAGE_W - approx_w) / 2.0).max(MARGIN);
            self.left(&line, font, size, x);
        }
    }

    fn left(&mut self, text: &str, font: &'static str, size: f32, x: f32) {
        if self.y < MARGIN {
            self.new_page();
        }
        let y = self.y;
        self.current_page().push(TextLine {
            x,
            y,
            font,
            size,
            text: pdf_text(text),
        });
        self.y -= LINE_H;
    }

    fn new_page(&mut self) {
        self.pages.push(Vec::new());
        self.y = PAGE_H - MARGIN;
        let page_num = self.pages.len().to_string();
        let x = PAGE_W - MARGIN - 28.0;
        let y = PAGE_H - 54.0;
        self.current_page().push(TextLine {
            x,
            y,
            font: "F1",
            size: FONT_SIZE,
            text: page_num,
        });
        self.y -= LINE_H * 2.0;
    }

    fn current_page(&mut self) -> &mut Vec<TextLine> {
        self.pages.last_mut().expect("page exists")
    }
}

fn render_pdf(pages: &[Vec<TextLine>]) -> Vec<u8> {
    let font1 = 3usize;
    let font2 = 4usize;
    let mut objects = vec![
        String::from("<< /Type /Catalog /Pages 2 0 R >>"),
        String::new(),
        String::from("<< /Type /Font /Subtype /Type1 /BaseFont /Times-Roman >>"),
        String::from("<< /Type /Font /Subtype /Type1 /BaseFont /Times-Bold >>"),
    ];
    let mut page_ids = Vec::new();
    for page in pages {
        let page_id = objects.len() + 1;
        let content_id = page_id + 1;
        page_ids.push(page_id);
        objects.push(format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {PAGE_W:.0} {PAGE_H:.0}] /Resources << /Font << /F1 {font1} 0 R /F2 {font2} 0 R >> >> /Contents {content_id} 0 R >>"
        ));
        let stream = page_stream(page);
        objects.push(format!(
            "<< /Length {} >>\nstream\n{}endstream",
            stream.len(),
            stream
        ));
    }
    let kids = page_ids
        .iter()
        .map(|id| format!("{id} 0 R"))
        .collect::<Vec<_>>()
        .join(" ");
    objects[1] = format!("<< /Type /Pages /Kids [{kids}] /Count {} >>", pages.len());

    let mut out = Vec::new();
    out.extend_from_slice(b"%PDF-1.4\n");
    let mut offsets = Vec::new();
    for (idx, object) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n{}\nendobj\n", idx + 1, object).as_bytes());
    }
    let xref = out.len();
    out.extend_from_slice(
        format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes(),
    );
    for offset in offsets {
        out.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            objects.len() + 1
        )
        .as_bytes(),
    );
    out
}

fn page_stream(lines: &[TextLine]) -> String {
    let mut stream = String::new();
    for line in lines {
        stream.push_str(&format!(
            "BT /{} {:.1} Tf {:.1} {:.1} Td ({}) Tj ET\n",
            line.font,
            line.size,
            line.x,
            line.y,
            escape_pdf_literal(&line.text)
        ));
    }
    stream
}

fn paragraphs(text: &str) -> Vec<String> {
    text.split("\n\n")
        .map(|p| p.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|p| !p.is_empty())
        .collect()
}

fn heading_title(line: &str) -> Option<String> {
    let trimmed = line.trim_start();
    let hashes = trimmed.chars().take_while(|&c| c == '#').count();
    if hashes == 0 {
        return None;
    }
    Some(
        trimmed[hashes..]
            .trim()
            .trim_matches('#')
            .trim()
            .to_string(),
    )
}

fn wrap_text(text: &str, width: f32, size: f32) -> Vec<String> {
    let max_chars = ((width / (size * 0.5)).floor() as usize).max(20);
    let mut lines = Vec::new();
    let mut current = String::new();
    for word in text.split_whitespace() {
        let next_len = current.len() + usize::from(!current.is_empty()) + word.len();
        if next_len > max_chars && !current.is_empty() {
            lines.push(current);
            current = String::new();
        }
        if !current.is_empty() {
            current.push(' ');
        }
        current.push_str(word);
    }
    if !current.is_empty() {
        lines.push(current);
    }
    lines
}

fn escape_pdf_literal(text: &str) -> String {
    text.replace('\\', "\\\\")
        .replace('(', "\\(")
        .replace(')', "\\)")
}

fn pdf_text(text: &str) -> String {
    let mut out = String::new();
    for c in text.chars() {
        match c {
            '\u{2018}' | '\u{2019}' => out.push('\''),
            '\u{201c}' | '\u{201d}' => out.push('"'),
            '\u{2013}' | '\u{2014}' => out.push('-'),
            '\u{2026}' => out.push_str("..."),
            c if c.is_ascii() && !c.is_control() => out.push(c),
            _ => out.push('?'),
        }
    }
    out
}
