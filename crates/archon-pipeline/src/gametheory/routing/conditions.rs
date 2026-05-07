use super::super::fingerprint::GameTheoryFingerprint;

// ── Expression AST ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Value {
    String(String),
    Number(i64),
    Bool(bool),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CmpOp {
    Eq,
    Neq,
    Lt,
    Gt,
    Lte,
    Gte,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Expr {
    Comparison {
        ident: String,
        op: CmpOp,
        value: Value,
    },
    And(Box<Expr>, Box<Expr>),
    Or(Box<Expr>, Box<Expr>),
    Not(Box<Expr>),
}

// ── Tokenizer ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
enum Token {
    Ident(String),
    StringLit(String),
    Number(i64),
    Eq,
    Neq,
    Lt,
    Gt,
    Lte,
    Gte,
    And,
    Or,
    Not,
    LParen,
    RParen,
}

struct Tokenizer {
    chars: Vec<char>,
    pos: usize,
}

impl Tokenizer {
    fn new(input: &str) -> Self {
        Self {
            chars: input.chars().collect(),
            pos: 0,
        }
    }

    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn advance(&mut self) -> Option<char> {
        let c = self.peek();
        if c.is_some() {
            self.pos += 1;
        }
        c
    }

    fn skip_whitespace(&mut self) {
        while let Some(c) = self.peek() {
            if c.is_whitespace() {
                self.advance();
            } else {
                break;
            }
        }
    }

    fn read_string(&mut self) -> Result<String, String> {
        self.advance(); // skip opening quote
        let mut s = String::new();
        while let Some(c) = self.advance() {
            if c == '\'' {
                return Ok(s);
            }
            s.push(c);
        }
        Err("unterminated string literal".into())
    }

    fn read_ident_or_number(&mut self) -> Token {
        let mut s = String::new();
        while let Some(c) = self.peek() {
            if c.is_alphanumeric() || c == '_' || c == '.' {
                s.push(c);
                self.advance();
            } else {
                break;
            }
        }
        // If all chars are digits, treat as number
        if s.chars().all(|c| c.is_ascii_digit()) {
            return Token::Number(s.parse().unwrap());
        }
        // Check for bool literals
        if s == "true" {
            return Token::Ident("true".into());
        }
        if s == "false" {
            return Token::Ident("false".into());
        }
        Token::Ident(s)
    }

    fn tokenize(&mut self) -> Result<Vec<Token>, String> {
        let mut tokens = Vec::new();
        loop {
            self.skip_whitespace();
            match self.peek() {
                None => break,
                Some('\'') => {
                    let s = self.read_string()?;
                    tokens.push(Token::StringLit(s));
                }
                Some('&') => {
                    self.advance();
                    if self.peek() == Some('&') {
                        self.advance();
                        tokens.push(Token::And);
                    } else {
                        return Err("expected '&&'".into());
                    }
                }
                Some('|') => {
                    self.advance();
                    if self.peek() == Some('|') {
                        self.advance();
                        tokens.push(Token::Or);
                    } else {
                        return Err("expected '||'".into());
                    }
                }
                Some('!') => {
                    self.advance();
                    if self.peek() == Some('=') {
                        self.advance();
                        tokens.push(Token::Neq);
                    } else {
                        tokens.push(Token::Not);
                    }
                }
                Some('=') => {
                    self.advance();
                    if self.peek() == Some('=') {
                        self.advance();
                        tokens.push(Token::Eq);
                    } else {
                        return Err("expected '=='".into());
                    }
                }
                Some('<') => {
                    self.advance();
                    if self.peek() == Some('=') {
                        self.advance();
                        tokens.push(Token::Lte);
                    } else {
                        tokens.push(Token::Lt);
                    }
                }
                Some('>') => {
                    self.advance();
                    if self.peek() == Some('=') {
                        self.advance();
                        tokens.push(Token::Gte);
                    } else {
                        tokens.push(Token::Gt);
                    }
                }
                Some('(') => {
                    self.advance();
                    tokens.push(Token::LParen);
                }
                Some(')') => {
                    self.advance();
                    tokens.push(Token::RParen);
                }
                Some(c) if c.is_alphanumeric() || c == '_' => {
                    tokens.push(self.read_ident_or_number());
                }
                Some(c) => {
                    return Err(format!("unexpected character: '{}'", c));
                }
            }
        }
        Ok(tokens)
    }
}

// ── Parser (recursive descent) ─────────────────────────────────────────────

struct Parser {
    tokens: Vec<Token>,
    pos: usize,
}

impl Parser {
    fn new(tokens: Vec<Token>) -> Self {
        Self { tokens, pos: 0 }
    }

    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.pos)
    }

    fn advance(&mut self) -> Option<Token> {
        let t = self.tokens.get(self.pos).cloned();
        if t.is_some() {
            self.pos += 1;
        }
        t
    }

    // expr = or_expr
    fn parse_expr(&mut self) -> Result<Expr, String> {
        self.parse_or()
    }

    // or_expr = and_expr ("||" and_expr)*
    fn parse_or(&mut self) -> Result<Expr, String> {
        let mut left = self.parse_and()?;
        while self.peek() == Some(&Token::Or) {
            self.advance();
            let right = self.parse_and()?;
            left = Expr::Or(Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    // and_expr = not_expr ("&&" not_expr)*
    fn parse_and(&mut self) -> Result<Expr, String> {
        let mut left = self.parse_not()?;
        while self.peek() == Some(&Token::And) {
            self.advance();
            let right = self.parse_not()?;
            left = Expr::And(Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    // not_expr = "!" not_expr | primary
    fn parse_not(&mut self) -> Result<Expr, String> {
        if self.peek() == Some(&Token::Not) {
            self.advance();
            let inner = self.parse_not()?;
            Ok(Expr::Not(Box::new(inner)))
        } else {
            self.parse_primary()
        }
    }

    // primary = comparison | "(" expr ")"
    fn parse_primary(&mut self) -> Result<Expr, String> {
        if self.peek() == Some(&Token::LParen) {
            self.advance();
            let expr = self.parse_expr()?;
            if self.peek() != Some(&Token::RParen) {
                return Err("expected ')'".into());
            }
            // Don't consume RParen here — let the caller handle it
            // Actually we need to consume it
            self.advance();
            Ok(expr)
        } else {
            self.parse_comparison()
        }
    }

    // comparison = identifier op value
    fn parse_comparison(&mut self) -> Result<Expr, String> {
        let ident = match self.advance() {
            Some(Token::Ident(id)) => id.clone(),
            Some(t) => return Err(format!("expected identifier, found {:?}", t)),
            None => return Err("expected identifier, found end".into()),
        };

        let op = match self.advance() {
            Some(Token::Eq) => CmpOp::Eq,
            Some(Token::Neq) => CmpOp::Neq,
            Some(Token::Lt) => CmpOp::Lt,
            Some(Token::Gt) => CmpOp::Gt,
            Some(Token::Lte) => CmpOp::Lte,
            Some(Token::Gte) => CmpOp::Gte,
            Some(ref t) => return Err(format!("expected comparison operator, found {:?}", t)),
            None => return Err("expected comparison operator, found end".into()),
        };

        let value = match self.advance() {
            Some(Token::StringLit(s)) => Value::String(s.clone()),
            Some(Token::Number(n)) => Value::Number(n),
            Some(Token::Ident(id)) if id == "true" => Value::Bool(true),
            Some(Token::Ident(id)) if id == "false" => Value::Bool(false),
            Some(t) => return Err(format!("expected value, found {:?}", t)),
            None => return Err("expected value, found end".into()),
        };

        Ok(Expr::Comparison { ident, op, value })
    }
}

// ── Evaluator ──────────────────────────────────────────────────────────────

/// Context available during condition evaluation.
pub(super) struct EvalContext<'a> {
    pub(super) fingerprint: &'a GameTheoryFingerprint,
    /// Number of specialists with "nash" in their key enabled so far.
    pub(super) nash_count: i64,
    /// Total number of specialists enabled so far.
    pub(super) enabled_count: i64,
}

impl<'a> EvalContext<'a> {
    fn resolve(&self, ident: &str) -> Result<Value, String> {
        match ident {
            "nash_count" => Ok(Value::Number(self.nash_count)),
            "enabled_count" => Ok(Value::Number(self.enabled_count)),
            ident if ident.starts_with("fingerprint.") => {
                let axis = &ident["fingerprint.".len()..];
                self.resolve_axis(axis)
            }
            _ => Err(format!("unknown identifier: {}", ident)),
        }
    }

    fn resolve_axis(&self, axis: &str) -> Result<Value, String> {
        let fp = self.fingerprint;
        let s: &str = match axis {
            "cooperation" => &fp.cooperation.value,
            "payoff_sum" => &fp.payoff_sum.value,
            "symmetry" => &fp.symmetry.value,
            "timing" => &fp.timing.value,
            "perfect_info" => &fp.perfect_info.value,
            "complete_info" => &fp.complete_info.value,
            "cardinality" => &fp.cardinality.value,
            "strategy_space" => &fp.strategy_space.value,
            "horizon" => &fp.horizon.value,
            "primary_family" => &fp.primary_family,
            "shadow_games_count" => {
                return Ok(Value::Number(fp.shadow_games.len() as i64));
            }
            _ => return Err(format!("unknown fingerprint axis: {}", axis)),
        };
        Ok(Value::String(s.to_string()))
    }
}

pub(super) fn eval_expr(expr: &Expr, ctx: &EvalContext<'_>) -> Result<bool, String> {
    match expr {
        Expr::Comparison { ident, op, value } => {
            let left = ctx.resolve(ident)?;
            Ok(cmp(&left, op, value))
        }
        Expr::And(a, b) => Ok(eval_expr(a, ctx)? && eval_expr(b, ctx)?),
        Expr::Or(a, b) => Ok(eval_expr(a, ctx)? || eval_expr(b, ctx)?),
        Expr::Not(inner) => Ok(!eval_expr(inner, ctx)?),
    }
}

fn cmp(left: &Value, op: &CmpOp, right: &Value) -> bool {
    match (left, right) {
        (Value::String(l), Value::String(r)) => match op {
            CmpOp::Eq => l == r,
            CmpOp::Neq => l != r,
            CmpOp::Lt => l < r,
            CmpOp::Gt => l > r,
            CmpOp::Lte => l <= r,
            CmpOp::Gte => l >= r,
        },
        (Value::Number(l), Value::Number(r)) => match op {
            CmpOp::Eq => l == r,
            CmpOp::Neq => l != r,
            CmpOp::Lt => l < r,
            CmpOp::Gt => l > r,
            CmpOp::Lte => l <= r,
            CmpOp::Gte => l >= r,
        },
        (Value::Bool(l), Value::Bool(r)) => match op {
            CmpOp::Eq => l == r,
            CmpOp::Neq => l != r,
            _ => false,
        },
        // Cross-type comparisons always false
        _ => match op {
            CmpOp::Neq => true,
            _ => false,
        },
    }
}

/// Parse a condition expression string into an AST.
pub(crate) fn parse_condition(expr_str: &str) -> Result<Expr, String> {
    let mut tokenizer = Tokenizer::new(expr_str);
    let tokens = tokenizer.tokenize()?;
    let mut parser = Parser::new(tokens);
    let expr = parser.parse_expr()?;
    if parser.peek().is_some() {
        return Err(format!(
            "unexpected token after expression: {:?}",
            parser.peek()
        ));
    }
    Ok(expr)
}
