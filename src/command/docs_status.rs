use anyhow::Result;
use cozo::DbInstance;

pub(crate) async fn handle_status(db: DbInstance) -> Result<()> {
    let summary = archon_docs::status::get_status_summary(&db)?;
    println!("Total sources:   {}", summary.total_sources);
    println!("  Discovered:    {}", summary.discovered);
    println!("  Ingesting:     {}", summary.ingesting);
    println!("  Ingested:      {}", summary.ingested);
    println!("  Processing:    {}", summary.processing);
    println!("  Processed:     {}", summary.processed);
    println!("  Failed:        {}", summary.failed);
    println!("Total chunks:    {}", summary.total_chunks);
    println!("Total pages:     {}", summary.total_pages);
    println!(
        "PDF images:      {} extracted",
        summary.pdf_embedded_images_extracted
    );
    println!(
        "PDF image skips: {} filtered",
        summary.pdf_embedded_images_skipped_filter
    );
    println!(
        "PDF image OCR:   {} run(s), {} failed",
        summary.pdf_image_ocr_runs, summary.pdf_image_ocr_failures
    );
    println!(
        "PDF image VLM:   {} description(s), {} failed",
        summary.pdf_image_vlm_descriptions, summary.pdf_image_vlm_failures
    );
    println!("PDF rendered:    {} page(s)", summary.pdf_pages_rendered);
    match archon_docs::index_queue::stats(&db) {
        Ok(queue) => {
            println!("Index queue:");
            println!("  Pending:     {}", queue.pending);
            println!("  Leased:      {}", queue.leased);
            println!("  Indexed:     {}", queue.indexed);
            println!("  Failed:      {}", queue.failed);
        }
        Err(e) => println!("Index queue:   unavailable — {e}"),
    }
    match archon_docs::index_jobs::summary(&db) {
        Ok(jobs) => println!(
            "Index jobs:     {} running, {} paused, {} completed, {} failed, {} cancelled",
            jobs.running, jobs.paused, jobs.completed, jobs.failed, jobs.cancelled
        ),
        Err(e) => println!("Index jobs:    unavailable — {e}"),
    }
    Ok(())
}
