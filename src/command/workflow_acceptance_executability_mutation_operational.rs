use super::*;

/// Programs whose first operand is the script they run.
const INTERPRETERS: [&str; 18] = [
    "sh", "bash", "zsh", "dash", "ksh", "python", "python2", "python3", "node", "deno", "bun",
    "ruby", "perl", "php", "Rscript", "lua", "tclsh", "pwsh",
];
/// Words that precede the command word without being it.
const PREFIXES: [&str; 22] = [
    "if", "then", "else", "elif", "fi", "do", "done", "while", "until", "for", "case", "esac",
    "in", "!", "exec", "env", "time", "command", "nohup", "xargs", "timeout", "nice",
];
/// Options whose value is a directory or manifest the command works from.
const LOCATION_FLAGS: [&str; 6] = [
    "-C",
    "--manifest-path",
    "--directory",
    "--cwd",
    "--project",
    "--prefix",
];
/// The words of `text` that make the check run (see the module docs).
pub(super) fn operational_words(commands: &[Vec<String>]) -> BTreeSet<String> {
    let mut excluded = BTreeSet::new();
    for words in commands {
        if words
            .iter()
            .any(|word| matches!(word.as_str(), paths::SUBSHELL_OPEN | paths::SUBSHELL_CLOSE))
        {
            continue;
        }
        let mut rest = words
            .iter()
            .skip_while(|word| {
                PREFIXES.contains(&word.as_str())
                    || matches!(word.as_str(), "\0and" | "\0or")
                    || word.starts_with('\0')
                    || word.starts_with('-')
                    || (word.contains('=') && !word.starts_with('='))
            })
            .peekable();
        let Some(command) = rest.next() else {
            continue;
        };
        excluded.insert(command.clone());
        let program = Path::new(command)
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        let operands: Vec<&String> = rest.collect();
        if program == "cd" || program == "pushd" {
            excluded.extend(
                operands
                    .iter()
                    .find(|word| word.as_str() == "-" || !word.starts_with('-'))
                    .map(|word| word.to_string()),
            );
        }
        if INTERPRETERS.contains(&program.as_str()) {
            for word in &operands {
                let inline = word.starts_with('-')
                    && !word.starts_with("--")
                    && (word.contains('c') || word.contains('e'));
                if inline || word.as_str() == "-" || word.as_str() == "-s" {
                    break;
                }
                if !word.starts_with('-') {
                    excluded.insert(word.to_string());
                    break;
                }
            }
        }
        for pair in operands.windows(2) {
            if LOCATION_FLAGS.contains(&pair[0].as_str()) {
                excluded.insert(pair[1].to_string());
            }
        }
        for word in &operands {
            if let Some((flag, value)) = word.split_once('=')
                && LOCATION_FLAGS.contains(&flag)
            {
                excluded.insert(value.to_string());
            }
        }
    }
    excluded
}
