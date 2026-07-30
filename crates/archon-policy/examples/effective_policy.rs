//! `cargo run -p archon-policy --example effective_policy` — load the *effective* policy for the
//! current working directory (system -> user -> workspace `.archon/policy.toml`) and print the
//! resolved PDF/Marker settings. Use it to confirm a machine's `.archon/policy.toml` actually
//! enables device-adaptive Marker ingestion by DEFAULT (i.e. the config file is honored, not just
//! that `from_policy` works when handed a policy in code).

fn main() {
    let cwd = std::env::current_dir().expect("cwd");
    let pol = archon_policy::load_effective_policy(&cwd).expect("load effective policy");
    let pdf = &pol.docs.pdf;
    println!("workspace        = {}", cwd.display());
    println!("chunker          = {}", pdf.chunker);
    println!("token_aware?     = {}", pdf.use_token_aware_chunker());
    println!("marker_sidecar   = {:?}", pdf.marker_sidecar);
    println!("marker_python    = {:?}", pdf.marker_python);
    println!("marker_device    = {:?}", pdf.marker_device);
    match &pdf.marker_sidecar {
        Some(s) => println!(
            ">> Marker IS the default — `archon docs ingest` will run device-adaptive Marker + bbox ({s})"
        ),
        None => println!(
            ">> Marker NOT configured — ingest falls back to pdftotext (text only, no bboxes)"
        ),
    }
}
