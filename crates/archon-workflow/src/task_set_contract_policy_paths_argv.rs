//! Which arguments can be proved to use a simple command's directory.
use std::io::Read;
use std::path::Path;

pub(super) fn operands<'a>(
    segment: &'a [String],
    root: &Path,
    directory: Option<&Path>,
) -> &'a [String] {
    // These options can change the child's directory, including dispatched code.
    // They do not change the parent shell's directory for its next command.
    if segment.iter().any(|word| directory_option(word)) {
        return &[];
    }
    let Some(command) = target(segment) else {
        return &[];
    };
    let Some(program) = command.first() else {
        return &[];
    };
    let name = program.rsplit('/').next().unwrap_or(program);
    if matches!(name, "xargs" | "parallel" | "watch") {
        return &[];
    }
    let offset = segment.len() - command.len();
    if interpreter(name) {
        for (index, argument) in command.iter().enumerate().skip(1) {
            if matches!(
                argument.as_str(),
                "-c" | "-e" | "--eval" | "--command" | "--exec"
            ) {
                // The inline program has its own scanner; later arguments are
                // consumed by that program, rather than read by the interpreter.
                return &segment[..(offset + index + 2).min(segment.len())];
            }
            if !argument.starts_with('-') {
                // The script file itself is read here. Its later arguments may
                // be read after arbitrary directory changes inside the script.
                return &segment[..offset + index + 1];
            }
            if matches!(name, "sh" | "bash" | "dash" | "zsh" | "ksh")
                && !argument.starts_with("--")
                && argument.contains('c')
            {
                // Combined inline flags have not established a program boundary.
                return &[];
            }
        }
    } else if directory.is_some_and(|base| shebang(&root.join(base).join(program))) {
        return &segment[..offset + 1];
    }
    segment
}

fn directory_option(word: &str) -> bool {
    word.starts_with("-C")
        || matches!(word, "-execdir" | "-okdir")
        || word.strip_prefix("--").is_some_and(|option| {
            let name = option.split('=').next().unwrap_or(option);
            [
                "dir",
                "cwd",
                "chdir",
                "work-tree",
                "prefix",
                "manifest-path",
                "root",
            ]
            .iter()
            .any(|part| name.contains(part))
        })
}

pub(super) fn interpreter(name: &str) -> bool {
    name.starts_with("python")
        || matches!(
            name,
            "sh" | "bash"
                | "zsh"
                | "dash"
                | "ksh"
                | "node"
                | "ruby"
                | "perl"
                | "php"
                | "deno"
                | "bun"
                | "Rscript"
                | "julia"
        )
}

fn shebang(path: &Path) -> bool {
    if !std::fs::metadata(path).is_ok_and(|metadata| metadata.is_file()) {
        return false;
    }
    let mut prefix = [0; 2];
    std::fs::File::open(path)
        .and_then(|mut file| file.read_exact(&mut prefix))
        .is_ok()
        && prefix == *b"#!"
}

/// Unwrap only invocations whose options preserve the target's directory.
/// An unrecognized wrapper option hides its target rather than guessing it.
pub(super) fn target(mut segment: &[String]) -> Option<&[String]> {
    loop {
        let Some(program) = segment.first() else {
            return Some(segment);
        };
        if assignment(program) {
            segment = &segment[1..];
            continue;
        }
        let name = program.as_str();
        if !matches!(
            name,
            "env" | "timeout" | "nice" | "nohup" | "stdbuf" | "exec" | "command"
        ) {
            return Some(segment);
        }
        segment = &segment[1..];
        while let Some(option) = segment.first().filter(|word| word.starts_with('-')) {
            if option == "--" {
                segment = &segment[1..];
                break;
            }
            let takes_value = match name {
                "env" => matches!(option.as_str(), "-u" | "--unset" | "--argv0"),
                "timeout" => matches!(option.as_str(), "-s" | "-k" | "--signal" | "--kill-after"),
                "nice" => matches!(option.as_str(), "-n" | "--adjustment"),
                "stdbuf" => matches!(
                    option.as_str(),
                    "-i" | "-o" | "-e" | "--input" | "--output" | "--error"
                ),
                "exec" => option == "-a",
                _ => false,
            };
            if takes_value {
                segment = segment.get(2..)?;
                continue;
            }
            let flag = match name {
                "env" => {
                    matches!(
                        option.as_str(),
                        "-i" | "--ignore-environment" | "-0" | "--null" | "-v" | "--debug"
                    ) || option.starts_with("--unset=")
                        || option.starts_with("--argv0=")
                }
                "timeout" => {
                    matches!(
                        option.as_str(),
                        "--preserve-status" | "--foreground" | "--verbose"
                    ) || option.starts_with("--signal=")
                        || option.starts_with("--kill-after=")
                }
                "nice" => {
                    option.starts_with("--adjustment=")
                        || option
                            .strip_prefix('-')
                            .is_some_and(|value| value.parse::<i32>().is_ok())
                }
                "stdbuf" => ["-i", "-o", "-e", "--input=", "--output=", "--error="]
                    .iter()
                    .any(|prefix| option.starts_with(prefix)),
                "exec" => matches!(option.as_str(), "-c" | "-l"),
                "command" => option == "-p",
                _ => false,
            };
            if !flag {
                return None;
            }
            segment = &segment[1..];
        }
        if name == "timeout" {
            segment = segment.get(1..)?;
        }
    }
}

fn assignment(word: &str) -> bool {
    word.split_once('=').is_some_and(|(name, _)| {
        !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
    })
}

/// Only redirections and the closed set of direct path readers prove that a
/// relative argument is a filesystem requirement. Other argv words may be text
/// or be consumed by code with its own filesystem and directory semantics.
pub(super) fn path_operands(segment: &[String], operators: &[bool]) -> Vec<String> {
    let (command, mut paths) = without_redirections(segment, operators);
    let Some(command) = target(&command) else {
        return paths;
    };
    let Some(program) = command.first() else {
        return paths;
    };
    // A project program with the same basename is still opaque.
    let name = program.as_str();
    let arguments = &command[1..];
    if matches!(name, "cd" | "pushd") {
        if let [operand] = arguments
            && !operand.starts_with('-')
        {
            paths.push(operand.clone());
        }
        return paths;
    }
    if matches!(name, "test" | "[" | "[[") {
        paths.extend(arguments.windows(2).filter_map(|pair| {
            matches!(
                pair[0].as_str(),
                "-e" | "-f" | "-d" | "-r" | "-s" | "-x" | "-L" | "-h"
            )
            .then(|| pair[1].clone())
        }));
        return paths;
    }
    if !matches!(
        name,
        "cat"
            | "ls"
            | "stat"
            | "head"
            | "tail"
            | "wc"
            | "diff"
            | "cmp"
            | "cp"
            | "mv"
            | "ln"
            | "readlink"
            | "realpath"
            | "source"
            | "."
            | "file"
            | "du"
            | "find"
    ) {
        return paths;
    }
    let mut options = true;
    let mut arguments = arguments.iter();
    while let Some(word) = arguments.next() {
        if options && word == "--" {
            options = false;
            continue;
        }
        if name == "find" && (word.starts_with('-') || matches!(word.as_str(), "!" | "(")) {
            // Only start paths are inputs. Expression values and dispatched
            // argv have no path-reading meaning in the outer command.
            if paths.is_empty() && matches!(word.as_str(), "-H" | "-L" | "-P") {
                continue;
            }
            break;
        }
        if options && word.starts_with('-') {
            if option_value(name, word) {
                arguments.next();
            }
            continue;
        }
        paths.push(word.clone());
        if matches!(name, "source" | ".") {
            // Later arguments belong to the sourced program.
            break;
        }
    }
    paths
}

fn without_redirections(segment: &[String], operators: &[bool]) -> (Vec<String>, Vec<String>) {
    let mut command = Vec::new();
    let mut paths = Vec::new();
    let mut words = segment.iter().zip(operators);
    while let Some((word, operator)) = words.next() {
        if *operator && matches!(word.as_str(), "<" | ">" | ">>" | "1>" | "2>") {
            if let Some((path, _)) = words.next() {
                paths.push(path.clone());
            }
        } else {
            command.push(word.clone());
        }
    }
    (command, paths)
}

fn option_value(program: &str, word: &str) -> bool {
    match program {
        "head" | "tail" => matches!(
            word,
            "-n" | "-c"
                | "--lines"
                | "--bytes"
                | "--pid"
                | "--sleep-interval"
                | "--max-unchanged-stats"
        ),
        "ls" => matches!(
            word,
            "-I" | "--ignore"
                | "--hide"
                | "--block-size"
                | "--format"
                | "--time"
                | "--time-style"
                | "--sort"
                | "--color"
                | "--quoting-style"
                | "--tabsize"
                | "--width"
                | "-w"
                | "-T"
        ),
        "stat" => matches!(word, "-f" | "-c" | "--format" | "--printf"),
        "wc" => word == "--files0-from",
        "diff" => matches!(
            word,
            "-I" | "-x"
                | "-X"
                | "-S"
                | "-D"
                | "--ignore-matching-lines"
                | "--exclude"
                | "--exclude-from"
                | "--starting-file"
                | "--ifdef"
                | "--label"
        ),
        "cmp" => matches!(word, "-i" | "-n" | "--ignore-initial" | "--bytes"),
        "cp" | "mv" | "ln" => matches!(word, "-t" | "-S" | "--target-directory" | "--suffix"),
        "file" => matches!(
            word,
            "-f" | "-F"
                | "-m"
                | "-P"
                | "--files-from"
                | "--separator"
                | "--magic-file"
                | "--parameter"
        ),
        "du" => matches!(
            word,
            "-B" | "-d"
                | "-X"
                | "--block-size"
                | "--max-depth"
                | "--exclude"
                | "--exclude-from"
                | "--files0-from"
        ),
        _ => false,
    }
}
