use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use crate::command::workflow_host_command_logic_source::{
    Tok, attribute, is, skip_item, test_cfg, word,
};

#[path = "workflow_host_environment_name_lexer_tests.rs"]
mod name_lexer;
#[path = "workflow_host_environment_source_files_tests.rs"]
mod source_files;
#[path = "workflow_host_environment_source_resources_tests.rs"]
mod source_resources;

struct Source {
    path: PathBuf,
    tokens: Vec<Tok>,
    lines: Vec<usize>,
}

fn production_tokens(text: &str) -> Source {
    production_tokens_at(Path::new("fixture.rs"), text)
}

fn production_tokens_at(path: &Path, text: &str) -> Source {
    let (tokens, _, lines) = name_lexer::lex_names(text);
    let mut kept_lines = Vec::new();
    let mut kept = Vec::new();
    let mut bodies = Vec::new();
    let mut at = 0;
    while at < tokens.len() {
        if is(tokens.get(at), '#') {
            let start = at;
            let mut test = false;
            let inner = is(tokens.get(at + 1), '!');
            while is(tokens.get(at), '#') && is(tokens.get(at + 1), '!') == inner {
                let (attr, after) = attribute(&tokens, at);
                test |= test_cfg(attr) || matches!(attr, [Tok::Ident(name)] if name == "test");
                at = after;
            }
            if test {
                if inner {
                    let start = bodies.last().copied().unwrap_or(0);
                    kept.truncate(start);
                    kept_lines.truncate(start);
                    let mut depth = 0usize;
                    while at < tokens.len() {
                        match tokens[at] {
                            Tok::Punct('}') if depth == 0 => break,
                            Tok::Punct('{') => depth += 1,
                            Tok::Punct('}') => depth -= 1,
                            _ => {}
                        }
                        at += 1;
                    }
                    continue;
                }
                at = skip_item(&tokens, at);
            } else {
                // Keep production attributes and every platform's body.
                kept.extend_from_slice(&tokens[start..at]);
                kept_lines.extend_from_slice(&lines[start..at]);
            }
        } else {
            kept.push(tokens[at].clone());
            kept_lines.push(lines[at]);
            match tokens[at] {
                Tok::Punct('{') => bodies.push(kept.len()),
                Tok::Punct('}') => {
                    bodies.pop();
                }
                _ => {}
            }
            at += 1;
        }
    }
    Source {
        path: path.to_owned(),
        tokens: kept,
        lines: kept_lines,
    }
}

fn production_sources(base: &Path) -> Vec<Source> {
    source_resources::production_sources(base)
}

/// Names arriving only through configuration introduce no host-defined text.
/// A configured alias may also select an already known name; that remains valid.
/// Inspect every production source file and platform body, including
/// reads and constants: name ownership does not depend on a setter's shape.
/// Deliberate obfuscation through unresolved fragments that individually lack `ARCHON_` is outside this scan.
/// Includes built from `env!("OUT_DIR")` are derived from already scanned source (build.rs:94) and are excluded; nothing under target/ is scanned.
/// Every embedded shell name counts unless assigned locally in a script with no
/// export, declare, typeset, readonly, local, env, or set with a -/+ option word
/// outside comments and quoted heredoc bodies; any such word makes all names count.
fn owned_name_locations(sources: &[Source]) -> BTreeMap<String, BTreeSet<String>> {
    let mut names = BTreeMap::<String, BTreeSet<String>>::new();
    for source in sources {
        let tokens = &source.tokens;
        let mut at = 0;
        while at < tokens.len() {
            let (text, end) = if let Some(value) = literal_macro(tokens, at) {
                value
            } else if let Tok::Str(text) = &tokens[at] {
                (text.clone(), at + 1)
            } else {
                at += 1;
                continue;
            };
            let location = format!(
                "{}:{}",
                source.path.to_string_lossy().replace('\\', "/"),
                source.lines[at]
            );
            let prefix_query = at >= 3
                && is(tokens.get(at - 1), '(')
                && matches!(word(tokens.get(at - 2)), Some("starts_with" | "find"))
                && is(tokens.get(at - 3), '.')
                || is(tokens.get(end), '.')
                    && word(tokens.get(end + 1)) == Some("len")
                    && is(tokens.get(end + 2), '(')
                    && is(tokens.get(end + 3), ')');
            for (offset, _) in text.match_indices("ARCHON_") {
                let tail = &text[offset + "ARCHON_".len()..];
                let length = tail
                    .bytes()
                    .take_while(|byte| {
                        byte.is_ascii_uppercase() || byte.is_ascii_digit() || *byte == b'_'
                    })
                    .count();
                assert!(
                    prefix_query
                        || !(tail.starts_with('{')
                            || (length == 0 && tail.is_empty())
                            || tail[length..].starts_with('{')),
                    "{location}: runtime-built environment name: {text:?}"
                );
                // Prefix construction above is rejected even inside other text.
                // Complete names have identifier boundaries.
                if offset > 0 && identifier_byte(text.as_bytes()[offset - 1]) {
                    continue;
                }
                if length > 0 {
                    let name = text[offset..offset + "ARCHON_".len() + length].to_owned();
                    names.entry(name).or_default().insert(location.clone());
                }
            }
            at = end;
        }
    }
    names
}

fn identifier_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

// Only entirely literal concatenations are folded. Unresolved pieces remain
// visible to the prefix guard, so a partial name can never disappear.
fn literal_macro(tokens: &[Tok], at: usize) -> Option<(String, usize)> {
    if !is(tokens.get(at + 1), '!') {
        return None;
    }
    let close = match tokens.get(at + 2)? {
        Tok::Punct('(') => ')',
        Tok::Punct('[') => ']',
        Tok::Punct('{') => '}',
        _ => return None,
    };
    let mut cursor = at + 3;
    match word(tokens.get(at))? {
        "concat" => {
            let mut value = String::new();
            while !is(tokens.get(cursor), close) {
                let (part, end) = match tokens.get(cursor)? {
                    Tok::Str(text) => (text.clone(), cursor + 1),
                    _ => literal_macro(tokens, cursor)?,
                };
                value.push_str(&part);
                cursor = end;
                if is(tokens.get(cursor), ',') {
                    cursor += 1;
                } else if !is(tokens.get(cursor), close) {
                    return None;
                }
            }
            Some((value, cursor + 1))
        }
        "format" => {
            let Tok::Str(template) = tokens.get(cursor)? else {
                return None;
            };
            cursor += 1;
            let mut arguments = Vec::new();
            while is(tokens.get(cursor), ',') {
                cursor += 1;
                if is(tokens.get(cursor), close) {
                    break;
                }
                let (part, end) = match tokens.get(cursor)? {
                    Tok::Str(text) => (text.clone(), cursor + 1),
                    _ => literal_macro(tokens, cursor)?,
                };
                arguments.push(part);
                cursor = end;
            }
            if !is(tokens.get(cursor), close) {
                return None;
            }
            let mut value = String::new();
            let mut chars = template.chars().peekable();
            let mut arguments = arguments.into_iter();
            while let Some(ch) = chars.next() {
                match (ch, chars.peek()) {
                    ('{', Some('{')) | ('}', Some('}')) => {
                        chars.next();
                        value.push(ch);
                    }
                    ('{', Some('}')) => {
                        chars.next();
                        value.push_str(&arguments.next()?);
                    }
                    ('{' | '}', _) => return None,
                    _ => value.push(ch),
                }
            }
            if arguments.next().is_some() {
                return None;
            }
            Some((value, cursor + 1))
        }
        "stringify" => {
            let mut closes = vec![close];
            let mut value = String::new();
            let mut previous_word = false;
            while let Some(token) = tokens.get(cursor) {
                match token {
                    Tok::Punct('(') => closes.push(')'),
                    Tok::Punct('[') => closes.push(']'),
                    Tok::Punct('{') => closes.push('}'),
                    Tok::Punct(ch @ (')' | ']' | '}')) => {
                        if closes.pop()? != *ch {
                            return None;
                        }
                        if closes.is_empty() {
                            return Some((value, cursor + 1));
                        }
                    }
                    _ => {}
                }
                let current_word = matches!(token, Tok::Ident(_) | Tok::Str(_) | Tok::Number(_));
                // Adjacent identifiers stay separate; punctuation keeps group
                // boundaries visible without hiding a bare name prefix.
                if previous_word && current_word {
                    value.push(' ');
                }
                match token {
                    Tok::Ident(text) | Tok::Str(text) | Tok::Number(text) => value.push_str(text),
                    Tok::Punct(ch) => value.push(*ch),
                    Tok::PathSep => value.push_str("::"),
                }
                previous_word = current_word;
                cursor += 1;
            }
            None
        }
        _ => None,
    }
}

fn owned_names(sources: &[Source]) -> BTreeSet<String> {
    owned_name_locations(sources).into_keys().collect()
}

fn warnings(names: BTreeSet<String>) -> Vec<String> {
    let environment = names
        .into_iter()
        .map(|name| (name, "1".to_string()))
        .collect();
    let mut unrecognized = archon_core::env_vars::warn_unrecognized_archon_vars(
        &environment,
        &crate::setup::extra_known_env_vars(),
    );
    unrecognized.sort();
    unrecognized
}

fn extra_known_env_literal_locations(text: &str) -> BTreeMap<String, BTreeSet<String>> {
    let source = production_tokens_at(Path::new("src/setup.rs"), text);
    let start = source
        .tokens
        .windows(2)
        .position(|tokens| {
            word(tokens.first()) == Some("fn")
                && word(tokens.get(1)) == Some("extra_known_env_vars")
        })
        .expect("extra_known_env_vars function missing");
    let body = start
        + source.tokens[start..]
            .iter()
            .position(|token| matches!(token, Tok::Punct('{')))
            .expect("extra_known_env_vars body missing");
    let mut depth = 1;
    let mut end = body + 1;
    while depth > 0 {
        match source
            .tokens
            .get(end)
            .expect("unclosed extra_known_env_vars body")
        {
            Tok::Punct('{') => depth += 1,
            Tok::Punct('}') => depth -= 1,
            _ => {}
        }
        end += 1;
    }
    owned_name_locations(&[Source {
        path: source.path,
        tokens: source.tokens[body..end].to_vec(),
        lines: source.lines[body..end].to_vec(),
    }])
}

#[test]
fn extra_known_env_host_set_names_are_derived_from_production_sources() {
    let literals = extra_known_env_literal_locations(include_str!("../setup.rs"));
    assert!(
        literals.is_empty(),
        "extra_known_env_vars must reference production constants, found literals: {literals:#?}"
    );
    let base = Path::new(env!("CARGO_MANIFEST_DIR"));
    let sources = production_sources(base);
    let locations = owned_name_locations(&sources);
    let names = locations.keys().cloned().collect::<BTreeSet<_>>();
    eprintln!("host-defined names: {names:#?}");
    assert!(!names.is_empty(), "no host-defined names were discovered");
    let unrecognized = warnings(names);
    let diagnostics = unrecognized
        .iter()
        .map(|name| {
            format!(
                "{name}: {}",
                locations[name]
                    .iter()
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        })
        .collect::<Vec<_>>();
    assert!(
        unrecognized.is_empty(),
        "unknown host-defined names:\n{}",
        diagnostics.join("\n")
    );

    let mut future = sources;
    future.push(production_tokens(
        r#"
        const CHILD_ENV: &str = "ARCHON_X_001";
        #[cfg(windows)]
        fn child() { command.env(CHILD_ENV, "1"); }
        #[cfg(test)]
        fn helper() { command.env("ARCHON_X_002", "1"); }
        fn reader() { std::env::var("ARCHON_X_003"); }
        fn local_child() { let name = "ARCHON_X_004"; command.env(name, "1"); }
    "#,
    ));
    assert_eq!(
        warnings(owned_names(&future)),
        vec!["ARCHON_X_001", "ARCHON_X_003", "ARCHON_X_004"]
    );
}

#[path = "workflow_host_environment_name_origin_tests.rs"]
mod name_origin_tests;

#[test]
fn build_fingerprint_name_matches_the_compiled_value_and_build_script() {
    use crate::command::workflow_host_command_logic::{BUILD_FINGERPRINT_ENV, THIS_BUILD};
    // `env!` needs a literal: the production read site must name the
    // constant's value, and build.rs must export the value under it.
    let reader = include_str!("workflow_host_command_logic.rs");
    let read_site = format!("THIS_BUILD: &str = env!(\"{BUILD_FINGERPRINT_ENV}\");");
    assert!(
        reader.contains(&read_site),
        "THIS_BUILD must read env!(\"{BUILD_FINGERPRINT_ENV}\")"
    );
    let tokens: String = reader.split_whitespace().collect();
    assert_eq!(tokens.matches("env!(").count(), 1, "one compile-time read");
    assert_eq!(THIS_BUILD, env!("ARCHON_BUILD_FINGERPRINT"));
    let script = include_str!("../../build.rs");
    assert!(
        script.contains(&format!("cargo:rustc-env={BUILD_FINGERPRINT_ENV}=")),
        "build.rs must export {BUILD_FINGERPRINT_ENV}"
    );
}
