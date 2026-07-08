//! archon-fcdp — CLI for the FCDP drafting-protocol gates.
//!
//!   archon-fcdp measure <file...>                          # sentence axes + Lanham metrics JSON
//!   archon-fcdp gp-validate <pack.json>                    # gate G-P (exit 1 on errors)
//!   archon-fcdp substitute <draft> <pack.json> <out>       # «Qnn» substitution (exit 1 on unknown IDs)
//!   archon-fcdp ga-gate <text> <gate-config.json> [--chapter]   # gate G-A (exit 1 on hard fail)
//!
//! Orchestration (stages D1/D1.5/D2, judge gates G-E/G-G, provenance, R-loop)
//! lives in scripts/fcdp/ pending the `archon draft` subcommand wiring.

use archon_draft::*;
use std::process::exit;

fn today() -> String {
    std::env::var("FCDP_TODAY").unwrap_or_else(|_| {
        let out = std::process::Command::new("date").arg("+%F").output().expect("date");
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    })
}

fn load_pack(path: &str) -> (Pack, QuoteBank) {
    let raw = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("read {path}: {e}"));
    let pack: Pack = serde_json::from_str(&raw).unwrap_or_else(|e| panic!("pack parse: {e}"));
    let bank_path = std::path::Path::new(path).parent().unwrap().join(&pack.p4b_bank_path);
    let braw = std::fs::read_to_string(&bank_path)
        .unwrap_or_else(|e| panic!("read bank {}: {e}", bank_path.display()));
    let bank: QuoteBank = serde_json::from_str(&braw).unwrap_or_else(|e| panic!("bank parse: {e}"));
    (pack, bank)
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(|s| s.as_str()) {
        Some("measure") => {
            let raw = args[1..]
                .iter()
                .map(|f| std::fs::read_to_string(f).unwrap_or_else(|e| panic!("read {f}: {e}")))
                .collect::<Vec<_>>()
                .join("\n\n");
            let mut out = measure_text(&strip_markup(&raw));
            out["corpus_files"] = serde_json::json!(args[1..]);
            println!("{}", serde_json::to_string_pretty(&out).unwrap());
        }
        Some("gp-validate") => {
            let (pack, bank) = load_pack(&args[1]);
            let (errs, warns) = gp_validate(&pack, &bank, &today());
            for w in &warns {
                eprintln!("WARN  {w}");
            }
            for e in &errs {
                eprintln!("ERROR {e}");
            }
            if errs.is_empty() {
                println!("G-P PASS ({} quotes, {} evidence items, {} exemplars)",
                    pack.p4a_quote_index.len(), pack.p5_evidence.len(), pack.p2b_exemplars.len());
            } else {
                println!("G-P FAIL — {} error(s)", errs.len());
                exit(1);
            }
        }
        Some("substitute") => {
            let draft = std::fs::read_to_string(&args[1]).expect("read draft");
            let (_, bank) = load_pack(&args[2]);
            let r = substitute_quote_ids(&draft, &bank);
            std::fs::write(&args[3], &r.output).expect("write out");
            if !r.unused.is_empty() {
                eprintln!("UNUSED bank entries: {}", r.unused.join(", "));
            }
            if !r.unknown.is_empty() {
                eprintln!("GATE FAIL — unknown quote IDs in draft: {}", r.unknown.join(", "));
                exit(1);
            }
            println!("OK — {} quotes substituted; {} bank entries unused", r.used.len(), r.unused.len());
        }
        Some("ga-gate") => {
            let chapter = args.iter().any(|a| a == "--chapter");
            let raw = std::fs::read_to_string(&args[1]).expect("read text");
            let metrics = measure_text(&strip_markup(&raw));
            let craw = std::fs::read_to_string(&args[2]).expect("read gate config");
            let cfg: GateConfig = serde_json::from_str(&craw).expect("gate config parse");
            let rep = ga_compare(&metrics, &cfg, chapter);
            println!("{}", serde_json::to_string_pretty(&rep).unwrap());
            if !rep.pass {
                exit(1);
            }
        }
        _ => {
            eprintln!("usage: archon-fcdp measure|gp-validate|substitute|ga-gate ...");
            exit(2);
        }
    }
}
