//! Here-doc delimiters use quote removal; their bodies are never commands.

#[derive(Default)]
pub(super) struct HereDocs(Vec<(Vec<u8>, bool)>);

impl HereDocs {
    pub(super) fn read(&mut self, input: &[u8], at: &mut usize) -> bool {
        let tabs = input.get(*at) == Some(&b'-');
        *at += usize::from(tabs);
        while matches!(input.get(*at), Some(b' ' | b'\t')) {
            *at += 1;
        }
        let start = *at;
        let mut delimiter = Vec::new();
        let mut quote = None;
        while let Some(&c) = input.get(*at) {
            if quote.is_none() && (c.is_ascii_whitespace() || b";&|<>()".contains(&c)) {
                break;
            }
            *at += 1;
            match (quote, c) {
                (Some(q), c) if q == c => quote = None,
                (None, b'\'' | b'"') => quote = Some(c),
                (None | Some(b'"'), b'\\') => {
                    let Some(&next) = input.get(*at) else {
                        return false;
                    };
                    if quote.is_none() || b"$`\"\\\n".contains(&next) {
                        *at += 1;
                        if next != b'\n' {
                            delimiter.push(next);
                        }
                    } else {
                        delimiter.push(c);
                    }
                }
                _ => delimiter.push(c),
            }
        }
        if *at == start || quote.is_some() {
            return false;
        }
        self.0.push((delimiter, tabs));
        true
    }

    pub(super) fn skip(&mut self, input: &[u8], at: &mut usize) -> bool {
        for (delimiter, tabs) in self.0.drain(..) {
            loop {
                if *at == input.len() {
                    return false;
                }
                let start = *at;
                while *at < input.len() && input[*at] != b'\n' {
                    *at += 1;
                }
                let mut line = &input[start..*at];
                *at += usize::from(*at < input.len());
                if tabs {
                    while line.first() == Some(&b'\t') {
                        line = &line[1..];
                    }
                }
                if line == delimiter {
                    break;
                }
            }
        }
        true
    }
}
