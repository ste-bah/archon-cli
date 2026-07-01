//! `cargo run -p archon-ingest-ext --example chunk_dump -- <marker.json>` — parse a Marker JSON
//! dump and run the token-aware chunker, printing one line per chunk (idx, page_start, page_end,
//! text_len, text_head). Mirrors `scripts/chunk_parity_check.py --marker-json` so the Rust port
//! can be diffed against the Python reference on real corpus PDFs (PR-D chunk parity).

use std::fs;

use archon_ingest_ext::chunk::chunk_blocks_default;
use archon_ingest_ext::marker::parse_marker_str;

fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("usage: chunk_dump <marker.json>");
    let json = fs::read_to_string(&path).expect("read marker json");
    let blocks = parse_marker_str(&json).expect("parse marker json");
    let chunks = chunk_blocks_default(&blocks);
    eprintln!("parsed {} blocks -> {} chunks", blocks.len(), chunks.len());
    println!("idx\tpage_start\tpage_end\ttext_len\ttext_head");
    for (i, c) in chunks.iter().enumerate() {
        let head: String = c
            .text
            .chars()
            .take(40)
            .collect::<String>()
            .replace('\n', " ");
        println!(
            "{i}\t{}\t{}\t{}\t{}",
            c.page_start,
            c.page_end,
            c.text.chars().count(),
            head
        );
    }
}
