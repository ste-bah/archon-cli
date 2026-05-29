use std::io::Write;

use crate::pdf::{PdfImage, PdfImageOrigin};

pub(crate) fn emit_pdf_image_progress(
    document_id: &str,
    current: usize,
    total: usize,
    image: &PdfImage,
    stage: &str,
    status: &str,
    detail: &str,
) {
    let pages = image
        .source_pages
        .iter()
        .map(u32::to_string)
        .collect::<Vec<_>>()
        .join(",");
    let origin = pdf_image_origin_label(&image.origin);
    let detail = if detail.trim().is_empty() {
        String::new()
    } else {
        format!(" detail={}", detail.replace('\n', " "))
    };
    emit_pdf_progress(format!(
        "PDF image progress: doc={document_id} image={current}/{total} page={} pages={} origin={origin} stage={stage} status={status}{detail}",
        image.source_page, pages
    ));
}

pub(crate) fn emit_pdf_progress(message: String) {
    tracing::info!(message = %message, "PDF image progress");
    let mut stderr = std::io::stderr().lock();
    let _ = writeln!(stderr, "{message}");
    let _ = stderr.flush();
}

fn pdf_image_origin_label(origin: &PdfImageOrigin) -> &'static str {
    match origin {
        PdfImageOrigin::Embedded { .. } => "embedded",
        PdfImageOrigin::RenderedPage => "rendered-page",
    }
}
