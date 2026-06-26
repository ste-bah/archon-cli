use clap::Subcommand;

#[derive(Subcommand, Debug, Clone)]
pub enum StyleAction {
    /// Train an output-style from sample prose by measuring its Lanham style
    /// (POS-free, fully offline). Writes a `.md` Archon output-style.
    Train {
        /// Sample text file(s) to learn the style from (omit to read stdin)
        files: Vec<String>,
        /// Name for the output-style (basename of the `.md` + style id)
        #[arg(long, default_value = "trained-style")]
        name: String,
        /// Genre register frame (academic, narrative, journalistic, technical, general)
        #[arg(long, default_value = "academic")]
        genre: String,
        /// Output path (default: ~/.archon/output-styles/<name>.md)
        #[arg(long)]
        out: Option<String>,
        /// Print the rendered output-style to stdout instead of writing a file
        #[arg(long)]
        stdout: bool,
    },
}
