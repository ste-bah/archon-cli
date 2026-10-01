//! Batch O2 (CUT-12): the files a brace pattern names, found WITHOUT
//! expanding it, for a piece whose brace sets expand past
//! [`super::BRACE_EXPANSION_BOUND`].
//!
//! The expansion names, per alternative `e`, exactly `matches(e)`. Here each
//! tree file is instead matched against the pattern itself, by a matcher
//! that chooses brace alternatives as it reads (`Program`), so the answer is
//! the one the full expansion gives -- only the work is bounded:
//!
//! - an alternative with a glob character names a file its glob matches, at
//!   the file's full path or after any `/` (as `matches` reads a glob);
//! - an alternative without one is a literal path: it can only name a file
//!   when it IS that file, one of its directories or one of its `/`
//!   suffixes, so each of those that the pattern spells is resolved by
//!   `matches` itself (exact file, directory, unique short form).

use std::collections::{BTreeSet, HashMap};
use std::path::Path;

use super::{clean_candidate, glob_free, matches};
use crate::v2::verification::path_ownership::{DeclaredPathForm, declared_path_form};

/// The files `piece` names in `files`, as its full expansion would.
pub(super) fn named_by(piece: &str, root: &Path, files: &BTreeSet<String>) -> BTreeSet<String> {
    let mut found = BTreeSet::new();
    // m1: a location suffix is taken off the piece's end, past its last
    // brace set only: one inside a brace set ends that expansion alone (the
    // matcher trims it there), and must not cut the set open.
    let core = match piece.rfind('}') {
        Some(at) => format!("{}{}", &piece[..=at], trim_location(&piece[at + 1..])),
        None => match super::super::residual_paths::strip_location(piece) {
            Some(core) => core.to_string(),
            None => return found,
        },
    };
    let core = core.strip_prefix("./").unwrap_or(&core);
    if !core.contains('/') || core.contains("://") {
        return found;
    }
    // m1: the piece is cleaned as a path WITHOUT the per-expansion checks
    // (`[`, `\\`, a leading `-`): those reject single alternatives, which the
    // matcher drops (`Op::Never`), or -- outside every brace -- every
    // expansion, which names nothing.
    let directory = core.ends_with('/');
    let mut pattern = match declared_path_form(core, root) {
        DeclaredPathForm::Repo(path) => path,
        _ => return found,
    };
    // An expansion's own trailing `/` makes it a directory; keep the one
    // the whole piece ends with (the path form trims it) for the matcher.
    if directory {
        pattern.push('/');
    }
    let Some(program) = Program::compile(&pattern) else {
        return found;
    };
    // An expansion's trailing `/` is trimmed before it is read, so a path
    // matches it with one appended too.
    let accepts = |text: &str, glob: bool| {
        program.accepts(text, glob) || program.accepts(&format!("{text}/"), glob)
    };
    for file in files {
        let suffixes = || file.match_indices('/').map(|(at, _)| &file[at + 1..]);
        if accepts(file, true) || suffixes().any(|tail| accepts(tail, true)) {
            found.insert(file.clone());
        }
        let mut literal: Vec<(&str, bool)> = vec![(file.as_str(), false)];
        literal.extend(suffixes().map(|tail| (tail, false)));
        for (at, _) in file.match_indices('/') {
            literal.push((&file[..at], false));
            literal.push((&file[..at], true));
        }
        for (candidate, as_directory) in literal {
            let spelled = if as_directory {
                program.accepts(&format!("{candidate}/"), false)
            } else {
                program.accepts(candidate, false)
            };
            if spelled && glob_free(candidate) && clean_candidate(candidate, root).is_some() {
                found.extend(matches(candidate, as_directory, files));
            }
        }
    }
    found
}

#[derive(Debug, Clone, Copy)]
enum Op {
    Byte(u8),
    /// `?`: one character, never `/`.
    One,
    /// `*`: any run within a segment.
    Star,
    /// `**`: any run across segments; a `/` right after it may be empty
    /// where it starts a segment (as `glob` reads it).
    Globstar,
    /// Either continue at `pc + 1` or jump to the target.
    Split(usize),
    /// An alternative every expansion through which is unclean.
    Never,
    Jump(usize),
    Accept,
}

struct Program(Vec<Op>);

impl Program {
    /// Brace sets exactly as `braces` reads them: the first `{`, the first
    /// `}` after it, alternatives split on `,`, taken literally.
    /// `None` when every expansion is unclean: a `[` or `\\` outside the
    /// braces, or a leading `-`.
    fn compile(pattern: &str) -> Option<Self> {
        let unclean = |text: &str| text.contains(['[', '\\']);
        let mut ops = Vec::new();
        let mut rest = pattern;
        while !rest.is_empty() {
            let open = rest.find('{');
            let close = open.and_then(|open| rest[open..].find('}').map(|at| open + at));
            let (Some(open), Some(close)) = (open, close) else {
                if unclean(rest) || (ops.is_empty() && rest.starts_with('-')) {
                    return None;
                }
                emit(&mut ops, rest);
                break;
            };
            let head = &rest[..open];
            if unclean(head) || (ops.is_empty() && head.starts_with('-')) {
                return None;
            }
            let at_start = ops.is_empty() && head.is_empty();
            emit(&mut ops, head);
            // m1: in the last brace set an alternative ends its expansion, so
            // its own location suffix or trailing punctuation is trimmed as
            // the expansion's would be (`strip_location`).
            let last = rest[close + 1..].is_empty();
            let alternatives: Vec<&str> = rest[open + 1..close]
                .split(',')
                .map(|alternative| {
                    if last {
                        trim_location(alternative)
                    } else {
                        alternative
                    }
                })
                .collect();
            let mut jumps = Vec::new();
            for (at, alternative) in alternatives.iter().enumerate() {
                let split = ops.len();
                if at + 1 < alternatives.len() {
                    ops.push(Op::Split(0));
                }
                if unclean(alternative) || (at_start && alternative.starts_with('-')) {
                    ops.push(Op::Never);
                } else {
                    emit(&mut ops, alternative);
                }
                if at + 1 < alternatives.len() {
                    jumps.push(ops.len());
                    ops.push(Op::Jump(0));
                    let next = ops.len();
                    ops[split] = Op::Split(next);
                }
            }
            let end = ops.len();
            for jump in jumps {
                ops[jump] = Op::Jump(end);
            }
            rest = &rest[close + 1..];
        }
        ops.push(Op::Accept);
        Some(Self(ops))
    }

    /// Whether some expansion matches `text`: one with a glob character,
    /// by glob, when `glob`; else one without, character for character.
    fn accepts(&self, text: &str, glob: bool) -> bool {
        let mut memo = HashMap::new();
        self.run(text.as_bytes(), 0, 0, false, false, glob, &mut memo)
    }

    #[allow(clippy::too_many_arguments)]
    fn run(
        &self,
        s: &[u8],
        pc: usize,
        sp: usize,
        used: bool,
        skip_slash: bool,
        glob: bool,
        memo: &mut HashMap<(usize, usize, bool, bool), bool>,
    ) -> bool {
        let key = (pc, sp, used, skip_slash);
        if let Some(known) = memo.get(&key) {
            return *known;
        }
        memo.insert(key, false);
        let next = |memo: &mut HashMap<_, _>, pc, sp, used, skip| {
            self.run(s, pc, sp, used, skip, glob, memo)
        };
        let at = s.get(sp).copied();
        let result = match self.0[pc] {
            Op::Accept => sp == s.len() && used == glob,
            Op::Byte(c) => {
                (at == Some(c) && next(memo, pc + 1, sp + 1, used, false))
                    || (c == b'/' && skip_slash && next(memo, pc + 1, sp, used, false))
            }
            Op::Split(other) => {
                next(memo, pc + 1, sp, used, skip_slash) || next(memo, other, sp, used, skip_slash)
            }
            Op::Jump(to) => next(memo, to, sp, used, skip_slash),
            Op::Never => false,
            _ if !glob => false,
            Op::One => at.is_some_and(|c| c != b'/') && next(memo, pc + 1, sp + 1, true, false),
            Op::Star => {
                next(memo, pc + 1, sp, true, false)
                    || (at.is_some_and(|c| c != b'/') && next(memo, pc, sp + 1, true, false))
            }
            Op::Globstar => {
                let starts_segment = sp == 0 || s[sp - 1] == b'/';
                next(memo, pc + 1, sp, true, starts_segment)
                    || (at.is_some() && next(memo, pc, sp + 1, true, false))
            }
        };
        memo.insert(key, result);
        result
    }
}

/// `alternative` less what `strip_location` takes off an expansion's end:
/// a `::symbol` or `#L` location, trailing sentence punctuation, and a
/// `:line`, `:line-line` or `:line:col` suffix.
fn trim_location(alternative: &str) -> &str {
    let mut token = alternative.split("::").next().unwrap_or(alternative);
    token = token.split("#L").next().unwrap_or(token);
    loop {
        let trimmed = token.trim_end_matches(['.', ',', ';', ':', '!', '?']);
        let stripped = match trimmed.rsplit_once(':') {
            Some((head, tail))
                if !tail.is_empty() && tail.bytes().all(|b| b.is_ascii_digit() || b == b'-') =>
            {
                head
            }
            _ => trimmed,
        };
        if stripped == token {
            return token;
        }
        token = stripped;
    }
}

fn emit(ops: &mut Vec<Op>, text: &str) {
    let bytes = text.as_bytes();
    let mut at = 0;
    while at < bytes.len() {
        match bytes[at] {
            b'*' if bytes.get(at + 1) == Some(&b'*') => {
                ops.push(Op::Globstar);
                at += 2;
                continue;
            }
            b'*' => ops.push(Op::Star),
            b'?' => ops.push(Op::One),
            c => ops.push(Op::Byte(c)),
        }
        at += 1;
    }
}
