//! `archon-style-train` — standalone trainer (identical logic to `archon style train`).
//!
//! Fully offline, all-Rust: text → full_analysis + base sentence/tone stats → profile →
//! output-style. No Node, no LLM, no network.
//!
//!   archon-style-train <file...> [--name NAME] [--genre academic] [--out PATH]
//!   (no files → read stdin; no --out → write to stdout)

use archon_lanham::train_to_output_style;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut files: Vec<String> = Vec::new();
    let mut name = "trained-style".to_string();
    let mut genre = "academic".to_string();
    let mut out: Option<String> = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--name" => { name = args.get(i + 1).cloned().unwrap_or_default(); i += 2; }
            "--genre" => { genre = args.get(i + 1).cloned().unwrap_or_else(|| "academic".into()); i += 2; }
            "--out" => { out = args.get(i + 1).cloned(); i += 2; }
            f if f.starts_with("--") => { i += 1; }
            _ => { files.push(args[i].clone()); i += 1; }
        }
    }

    let text = if files.is_empty() {
        use std::io::Read;
        let mut s = String::new();
        std::io::stdin().read_to_string(&mut s).expect("read stdin");
        s
    } else {
        files
            .iter()
            .map(|f| std::fs::read_to_string(f).unwrap_or_else(|e| panic!("read {f}: {e}")))
            .collect::<Vec<_>>()
            .join("\n\n")
    };

    let r = train_to_output_style(&text, &name, &genre);
    match out {
        Some(p) => {
            std::fs::write(&p, &r.md).unwrap_or_else(|e| panic!("write {p}: {e}"));
            eprintln!("trained '{name}' — voice={}, register={}, parataxis={} → {p}", r.voice, r.register, r.parataxis);
        }
        None => print!("{}", r.md),
    }
}
