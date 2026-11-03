use super::*;

/// Apply one-command `-C`/`--directory` scopes for common shell tools.
pub(crate) fn command_directories(
    commands: &[Vec<String>],
    mut bases: Vec<Option<PathBuf>>,
) -> Vec<Option<PathBuf>> {
    for (index, words) in commands.iter().enumerate() {
        let command_index = words
            .iter()
            .position(|word| {
                !word.starts_with('\0')
                    && !matches!(
                        word.as_str(),
                        "if" | "then"
                            | "else"
                            | "elif"
                            | "do"
                            | "while"
                            | "until"
                            | "!"
                            | "{"
                            | "("
                    )
            })
            .unwrap_or(0);
        let program = words
            .get(command_index)
            .and_then(|word| Path::new(word).file_name())
            .map(|name| name.to_string_lossy().into_owned());
        if !matches!(program.as_deref(), Some("env" | "git" | "make")) {
            continue;
        }
        let args = &words[command_index + 1..];
        let directory = args.iter().enumerate().find_map(|(i, word)| {
            if matches!(word.as_str(), "-C" | "--directory") {
                args.get(i + 1).cloned()
            } else {
                word.strip_prefix("--directory=").map(str::to_string)
            }
        });
        if let (Some(base), Some(directory)) = (bases.get(index).cloned().flatten(), directory) {
            let path = Path::new(&directory);
            bases[index] = Some(normalize_absolute(
                if path.is_absolute() {
                    path.to_path_buf()
                } else {
                    base.join(path)
                }
                .as_path(),
            ));
        }
    }
    bases
}
