//! `archon style` — train and manage prose output-styles via Lanham style analysis.
//!
//! `archon style train` measures the Lanham prose-style of sample text (POS-free,
//! fully offline) and renders an Archon output-style `.md` into ~/.archon/output-styles/.

use anyhow::{Context, Result};

use crate::cli_args::StyleAction;

pub(crate) async fn handle_style_command(action: StyleAction) -> Result<()> {
    match action {
        StyleAction::Train { files, name, genre, out, stdout } => {
            train(files, name, genre, out, stdout)
        }
    }
}

fn train(
    files: Vec<String>,
    name: String,
    genre: String,
    out: Option<String>,
    stdout: bool,
) -> Result<()> {
    let text = if files.is_empty() {
        use std::io::Read;
        let mut s = String::new();
        std::io::stdin()
            .read_to_string(&mut s)
            .context("reading sample text from stdin")?;
        s
    } else {
        let mut parts = Vec::with_capacity(files.len());
        for f in &files {
            parts.push(
                std::fs::read_to_string(f).with_context(|| format!("reading sample file {f}"))?,
            );
        }
        parts.join("\n\n")
    };

    if text.trim().is_empty() {
        anyhow::bail!("no sample text provided (pass file paths or pipe text on stdin)");
    }

    let result = archon_lanham::train_to_output_style(&text, &name, &genre);

    if stdout {
        print!("{}", result.md);
        return Ok(());
    }

    let path = match out {
        Some(p) => std::path::PathBuf::from(p),
        None => {
            let dir = dirs::home_dir()
                .context("could not resolve home directory for ~/.archon/output-styles")?
                .join(".archon")
                .join("output-styles");
            std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
            dir.join(format!("{name}.md"))
        }
    };
    std::fs::write(&path, &result.md).with_context(|| format!("writing {}", path.display()))?;
    println!(
        "trained '{name}' — voice={}, register={}, parataxis={} → {}",
        result.voice,
        result.register,
        result.parataxis,
        path.display()
    );
    println!(
        "activate it with  output_style = \"{name}\"  in your .archon config (or --output-style {name})"
    );
    Ok(())
}
