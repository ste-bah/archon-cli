//! Conditional specialist routing via expression evaluation.
//!
//! Parses `.archon/specs/gametheory.yaml`, evaluates per-agent condition
//! expressions against the Tier 1 fingerprint, and produces a deterministic
//! [`RoutingDecision`] with enabled/skipped specialists and cycle detection.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::path::PathBuf;

use serde::Deserialize;

use super::errors::GameTheoryError;
use super::fingerprint::GameTheoryFingerprint;

// ── YAML spec types ────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct GameTheorySpec {
    pub version: String,
    pub spec_id: String,
    #[serde(default)]
    pub cost_cap_usd: f64,
    pub tiers: Vec<TierEntry>,
}

#[derive(Debug, Deserialize)]
pub struct TierEntry {
    pub id: u8,
    pub name: String,
    #[serde(default = "default_concurrency")]
    pub concurrency_cap: usize,
    #[serde(default)]
    pub agents: Vec<AgentEntry>,
}

fn default_concurrency() -> usize {
    4
}

#[derive(Debug, Deserialize)]
pub struct AgentEntry {
    pub key: String,
    #[serde(default)]
    pub condition: Option<String>,
    #[serde(default)]
    pub mandatory: bool,
    #[serde(default)]
    pub depends_on: Vec<String>,
}

// ── RoutingDecision ────────────────────────────────────────────────────────

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RoutingDecision {
    pub run_id: String,
    pub fingerprint_id: String,
    pub enabled_specialists: Vec<String>,
    /// (agent_key, reason)
    pub skipped_specialists: Vec<(String, String)>,
    /// (expression, evaluated_result)
    pub evaluated_conditions: Vec<(String, bool)>,
    pub created_at: String,
}

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
struct EvalContext<'a> {
    fingerprint: &'a GameTheoryFingerprint,
    /// Number of specialists with "nash" in their key enabled so far.
    nash_count: i64,
    /// Total number of specialists enabled so far.
    enabled_count: i64,
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

fn eval_expr(expr: &Expr, ctx: &EvalContext) -> Result<bool, String> {
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

// ── Cycle detection ────────────────────────────────────────────────────────

/// Detect cycles in the dependency graph. Returns the cycle path if found.
fn detect_cycle(
    agent_key: &str,
    deps: &HashMap<String, Vec<String>>,
    visited: &mut HashSet<String>,
    path: &mut Vec<String>,
) -> Option<String> {
    if path.contains(&agent_key.to_string()) {
        // Build cycle description
        let cycle_start = path.iter().position(|k| k == agent_key).unwrap();
        let cycle: Vec<_> = path[cycle_start..].to_vec();
        let mut cycle_rep = cycle.join(" -> ");
        cycle_rep.push_str(" -> ");
        cycle_rep.push_str(agent_key);
        return Some(cycle_rep);
    }
    if visited.contains(agent_key) {
        return None;
    }
    visited.insert(agent_key.to_string());
    path.push(agent_key.to_string());

    if let Some(dep_list) = deps.get(agent_key) {
        for dep in dep_list {
            if let Some(cycle) = detect_cycle(dep, deps, visited, path) {
                return Some(cycle);
            }
        }
    }

    path.pop();
    None
}

// ── Public API ─────────────────────────────────────────────────────────────

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

/// Evaluate a routing specification against a fingerprint.
///
/// Processes tiers in order, evaluates each non-mandatory agent's condition,
/// and collects enabled/skipped specialists. Mandatory agents are always enabled.
/// Detects dependency cycles and returns deterministic results.
pub fn evaluate_routing(
    spec: &GameTheorySpec,
    fingerprint: &GameTheoryFingerprint,
    run_id: &str,
    created_at: &str,
) -> Result<RoutingDecision, GameTheoryError> {
    let mut enabled: Vec<String> = Vec::new();
    let mut enabled_set: HashSet<String> = HashSet::new();
    let mut skipped: Vec<(String, String)> = Vec::new();
    let mut evaluated: Vec<(String, bool)> = Vec::new();

    // Build dependency map for cycle detection
    let mut dep_map: HashMap<String, Vec<String>> = HashMap::new();
    for tier in &spec.tiers {
        for agent in &tier.agents {
            if !agent.depends_on.is_empty() {
                dep_map.insert(agent.key.clone(), agent.depends_on.clone());
            }
        }
    }

    // Check for cycles
    for key in dep_map.keys() {
        let mut visited = HashSet::new();
        let mut path = Vec::new();
        if let Some(cycle) = detect_cycle(key, &dep_map, &mut visited, &mut path) {
            return Err(GameTheoryError::RoutingCycle { cycle });
        }
    }

    // Also verify all dependency targets exist in the spec
    let all_keys: HashSet<&str> = spec
        .tiers
        .iter()
        .flat_map(|t| t.agents.iter().map(|a| a.key.as_str()))
        .collect();
    for tier in &spec.tiers {
        for agent in &tier.agents {
            for dep in &agent.depends_on {
                if !all_keys.contains(dep.as_str()) {
                    return Err(GameTheoryError::ConditionError {
                        expression: format!("depends_on: {}", dep),
                        message: format!(
                            "agent '{}' depends on '{}' which is not in the spec",
                            agent.key, dep
                        ),
                    });
                }
            }
        }
    }

    let mut enabled_count: i64 = 0;
    let mut nash_count: i64 = 0;

    // Process tiers in order
    for tier in &spec.tiers {
        for agent in &tier.agents {
            if agent.mandatory {
                if let Some(reason) = unmet_dependency_reason(agent, &enabled_set) {
                    skipped.push((agent.key.clone(), reason));
                } else {
                    enable_agent(
                        &agent.key,
                        &mut enabled,
                        &mut enabled_set,
                        &mut enabled_count,
                        &mut nash_count,
                    );
                }
                continue;
            }

            let condition = match &agent.condition {
                Some(c) => c.clone(),
                None => {
                    // No condition = enabled when its dependency chain is enabled.
                    if let Some(reason) = unmet_dependency_reason(agent, &enabled_set) {
                        skipped.push((agent.key.clone(), reason));
                    } else {
                        enable_agent(
                            &agent.key,
                            &mut enabled,
                            &mut enabled_set,
                            &mut enabled_count,
                            &mut nash_count,
                        );
                    }
                    continue;
                }
            };

            let ctx = EvalContext {
                fingerprint,
                nash_count,
                enabled_count,
            };

            match parse_condition(&condition).and_then(|expr| eval_expr(&expr, &ctx)) {
                Ok(true) => {
                    evaluated.push((condition.clone(), true));
                    if let Some(reason) = unmet_dependency_reason(agent, &enabled_set) {
                        skipped.push((agent.key.clone(), reason));
                    } else {
                        enable_agent(
                            &agent.key,
                            &mut enabled,
                            &mut enabled_set,
                            &mut enabled_count,
                            &mut nash_count,
                        );
                    }
                }
                Ok(false) => {
                    evaluated.push((condition.clone(), false));
                    skipped.push((agent.key.clone(), "condition evaluated to false".into()));
                }
                Err(e) => {
                    return Err(GameTheoryError::ConditionError {
                        expression: condition.clone(),
                        message: e,
                    });
                }
            }
        }
    }

    Ok(RoutingDecision {
        run_id: run_id.to_string(),
        fingerprint_id: fingerprint.run_id.clone(),
        enabled_specialists: enabled,
        skipped_specialists: skipped,
        evaluated_conditions: evaluated,
        created_at: created_at.to_string(),
    })
}

fn enable_agent(
    key: &str,
    enabled: &mut Vec<String>,
    enabled_set: &mut HashSet<String>,
    enabled_count: &mut i64,
    nash_count: &mut i64,
) {
    enabled.push(key.to_string());
    enabled_set.insert(key.to_string());
    *enabled_count += 1;
    if key.contains("nash") {
        *nash_count += 1;
    }
}

fn unmet_dependency_reason(agent: &AgentEntry, enabled_set: &HashSet<String>) -> Option<String> {
    let missing: Vec<&str> = agent
        .depends_on
        .iter()
        .filter(|dep| !enabled_set.contains(dep.as_str()))
        .map(String::as_str)
        .collect();

    if missing.is_empty() {
        None
    } else {
        Some(format!("dependency not enabled: {}", missing.join(", ")))
    }
}

/// Load the gametheory spec from the canonical YAML path.
pub fn load_spec(path: &Path) -> Result<GameTheorySpec, GameTheoryError> {
    let contents = std::fs::read_to_string(path).map_err(|e| GameTheoryError::Io {
        message: format!("cannot read spec at {}: {e}", path.display()),
    })?;
    serde_yml::from_str(&contents).map_err(|e| GameTheoryError::Validation {
        message: format!("invalid gametheory spec YAML: {e}"),
    })
}

/// Resolve the gametheory spec path by searching known locations.
///
/// Search order:
/// 1. Explicit `--spec-path` CLI flag (passed as `explicit_path`)
/// 2. `$ARCHON_SPEC_PATH` environment variable
/// 3. Walk up from CWD looking for `.archon/specs/gametheory.yaml` (max 5 levels)
/// 4. `~/.archon/specs/gametheory.yaml` (user install)
/// 5. `/etc/archon/specs/gametheory.yaml` (system install)
///
/// Returns the first path that exists, or a `SpecNotFound` error listing all
/// locations searched.
pub fn resolve_spec_path(
    explicit_path: Option<&Path>,
) -> Result<std::path::PathBuf, GameTheoryError> {
    let mut searched = Vec::new();

    // 1. Explicit CLI flag
    if let Some(p) = explicit_path {
        searched.push(p.to_path_buf());
        if p.exists() {
            return Ok(p.to_path_buf());
        }
    }

    // 2. Env var
    if let Ok(env_path) = std::env::var("ARCHON_SPEC_PATH") {
        let p = std::path::PathBuf::from(&env_path);
        searched.push(p.clone());
        if p.exists() {
            return Ok(p);
        }
    }

    // 3. Walk up from CWD (max 5 levels)
    if let Ok(cwd) = std::env::current_dir() {
        let mut current = cwd.as_path();
        for _ in 0..5 {
            let candidate = current.join(".archon/specs/gametheory.yaml");
            searched.push(candidate.clone());
            if candidate.exists() {
                return Ok(candidate);
            }
            match current.parent() {
                Some(parent) => current = parent,
                None => break,
            }
        }
    }

    // 4. User install
    if let Ok(home) = std::env::var("HOME") {
        let user_path = PathBuf::from(&home).join(".archon/specs/gametheory.yaml");
        searched.push(user_path.clone());
        if user_path.exists() {
            return Ok(user_path);
        }
    }

    // 5. System install
    let system_path = std::path::PathBuf::from("/etc/archon/specs/gametheory.yaml");
    searched.push(system_path.clone());
    if system_path.exists() {
        return Ok(system_path);
    }

    Err(GameTheoryError::SpecNotFound {
        searched_paths: searched,
    })
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gametheory::fingerprint::AxisVerdict;

    fn make_fingerprint(overrides: &[(&str, &str)]) -> GameTheoryFingerprint {
        let mut fp = GameTheoryFingerprint {
            run_id: "test-run-1".into(),
            cooperation: AxisVerdict::new("non-cooperative", "high", ""),
            payoff_sum: AxisVerdict::new("zero-sum", "medium", ""),
            symmetry: AxisVerdict::new("asymmetric", "medium", ""),
            timing: AxisVerdict::new("simultaneous", "high", ""),
            perfect_info: AxisVerdict::new("imperfect", "medium", ""),
            complete_info: AxisVerdict::new("incomplete", "medium", ""),
            cardinality: AxisVerdict::new("2-player", "high", ""),
            strategy_space: AxisVerdict::new("continuous", "high", ""),
            horizon: AxisVerdict::new("one-shot", "medium", ""),
            primary_family: "Cournot competition".into(),
            nearest_classic: Some("Cournot duopoly".into()),
            shadow_games: vec![],
            hidden_game_scan: None,
            ambiguities: vec![],
            created_at: "2026-05-03T00:00:00Z".into(),
        };
        for &(axis, value) in overrides {
            match axis {
                "cooperation" => fp.cooperation.value = value.into(),
                "payoff_sum" => fp.payoff_sum.value = value.into(),
                "symmetry" => fp.symmetry.value = value.into(),
                "timing" => fp.timing.value = value.into(),
                "perfect_info" => fp.perfect_info.value = value.into(),
                "complete_info" => fp.complete_info.value = value.into(),
                "cardinality" => fp.cardinality.value = value.into(),
                "strategy_space" => fp.strategy_space.value = value.into(),
                "horizon" => fp.horizon.value = value.into(),
                _ => {}
            }
        }
        fp
    }

    fn make_two_tier_spec(extra_agents: Vec<AgentEntry>) -> GameTheorySpec {
        GameTheorySpec {
            version: "1.0".into(),
            spec_id: "test-spec".into(),
            cost_cap_usd: 5.0,
            tiers: vec![
                TierEntry {
                    id: 1,
                    name: "Foundation".into(),
                    concurrency_cap: 4,
                    agents: vec![
                        AgentEntry {
                            key: "gt-payoff".into(),
                            condition: None,
                            mandatory: true,
                            depends_on: vec![],
                        },
                        AgentEntry {
                            key: "gt-classify".into(),
                            condition: None,
                            mandatory: true,
                            depends_on: vec![],
                        },
                    ],
                },
                TierEntry {
                    id: 2,
                    name: "Specialists".into(),
                    concurrency_cap: 3,
                    agents: extra_agents,
                },
            ],
        }
    }

    // ── Test 1: Grammar parsing ─────────────────────────────────────────

    #[test]
    fn test_routing_evaluator_parses_grammar() {
        let fp = make_fingerprint(&[]);

        // Exercise all operators: &&, ||, !, ==, !=, <, >, <=, >=
        let spec = make_two_tier_spec(vec![
            AgentEntry {
                key: "gt-and-test".into(),
                condition: Some(
                    "fingerprint.cooperation == 'non-cooperative' && fingerprint.strategy_space == 'continuous'"
                        .into(),
                ),
                mandatory: false,
                depends_on: vec![],
            },
            AgentEntry {
                key: "gt-or-test".into(),
                condition: Some(
                    "fingerprint.cooperation == 'cooperative' || fingerprint.payoff_sum == 'zero-sum'"
                        .into(),
                ),
                mandatory: false,
                depends_on: vec![],
            },
            AgentEntry {
                key: "gt-not-test".into(),
                condition: Some(
                    "! (fingerprint.cooperation == 'cooperative')"
                        .into(),
                ),
                mandatory: false,
                depends_on: vec![],
            },
            AgentEntry {
                key: "gt-neq-test".into(),
                condition: Some(
                    "fingerprint.cardinality != 'n-player'"
                        .into(),
                ),
                mandatory: false,
                depends_on: vec![],
            },
            AgentEntry {
                key: "gt-lt-test".into(),
                condition: Some(
                    "fingerprint.cardinality < 'n-player'"
                        .into(),
                ),
                mandatory: false,
                depends_on: vec![],
            },
            AgentEntry {
                key: "gt-gte-test".into(),
                condition: Some(
                    "fingerprint.cardinality >= '2-player'"
                        .into(),
                ),
                mandatory: false,
                depends_on: vec![],
            },
            AgentEntry {
                key: "gt-nash-count-test".into(),
                condition: Some("nash_count >= 0".into()),
                mandatory: false,
                depends_on: vec![],
            },
            AgentEntry {
                key: "gt-enabled-count-test".into(),
                condition: Some("enabled_count >= 2".into()),
                mandatory: false,
                depends_on: vec![],
            },
        ]);

        let decision = evaluate_routing(&spec, &fp, "test-run", "2026-01-01T00:00:00Z").unwrap();

        // All 8 test conditions should be evaluated
        assert_eq!(decision.evaluated_conditions.len(), 8);

        // && test: both true → enabled
        assert!(
            decision
                .enabled_specialists
                .contains(&"gt-and-test".to_string())
        );
        // || test: payoff_sum is zero-sum → enabled
        assert!(
            decision
                .enabled_specialists
                .contains(&"gt-or-test".to_string())
        );
        // ! test: cooperation is non-cooperative, so !(cooperative) → true → enabled
        assert!(
            decision
                .enabled_specialists
                .contains(&"gt-not-test".to_string())
        );
        // != test: 2-player != n-player → true → enabled
        assert!(
            decision
                .enabled_specialists
                .contains(&"gt-neq-test".to_string())
        );
        // < test: "2-player" < "n-player" lexicographically → true → enabled
        assert!(
            decision
                .enabled_specialists
                .contains(&"gt-lt-test".to_string())
        );
        // >= test: "2-player" >= "2-player" → true → enabled
        assert!(
            decision
                .enabled_specialists
                .contains(&"gt-gte-test".to_string())
        );
        // nash_count >= 0 → true → enabled
        assert!(
            decision
                .enabled_specialists
                .contains(&"gt-nash-count-test".to_string())
        );
        // enabled_count >= 2 (2 mandatory agents already counted) → true → enabled
        assert!(
            decision
                .enabled_specialists
                .contains(&"gt-enabled-count-test".to_string())
        );

        // 2 mandatory + 8 conditional = 10 enabled
        assert_eq!(decision.enabled_specialists.len(), 10);
    }

    // ── Test 2: Determinism ─────────────────────────────────────────────

    #[test]
    fn test_routing_deterministic_over_fixed_fingerprint() {
        let fp = make_fingerprint(&[]);
        let spec = make_two_tier_spec(vec![
            AgentEntry {
                key: "gt-cond-1".into(),
                condition: Some("fingerprint.cooperation == 'non-cooperative'".into()),
                mandatory: false,
                depends_on: vec![],
            },
            AgentEntry {
                key: "gt-cond-2".into(),
                condition: Some("fingerprint.payoff_sum == 'zero-sum'".into()),
                mandatory: false,
                depends_on: vec![],
            },
        ]);

        let first = evaluate_routing(&spec, &fp, "run-1", "2026-01-01T00:00:00Z").unwrap();
        for _ in 0..100 {
            let subsequent = evaluate_routing(&spec, &fp, "run-1", "2026-01-01T00:00:00Z").unwrap();
            assert_eq!(first.enabled_specialists, subsequent.enabled_specialists);
            assert_eq!(first.skipped_specialists, subsequent.skipped_specialists);
        }
    }

    #[test]
    fn test_routing_skips_agent_when_dependency_was_not_enabled() {
        let fp = make_fingerprint(&[]);
        let spec = make_two_tier_spec(vec![
            AgentEntry {
                key: "gt-disabled-dependency".into(),
                condition: Some("fingerprint.cooperation == 'cooperative'".into()),
                mandatory: false,
                depends_on: vec![],
            },
            AgentEntry {
                key: "gt-dependent".into(),
                condition: Some("fingerprint.payoff_sum == 'zero-sum'".into()),
                mandatory: false,
                depends_on: vec!["gt-disabled-dependency".into()],
            },
        ]);

        let decision = evaluate_routing(&spec, &fp, "run-deps", "2026-01-01T00:00:00Z").unwrap();

        assert!(
            !decision
                .enabled_specialists
                .contains(&"gt-dependent".to_string()),
            "dependent specialist must not run when its dependency was skipped"
        );
        assert!(
            decision.skipped_specialists.iter().any(|(key, reason)| {
                key == "gt-dependent" && reason.contains("gt-disabled-dependency")
            }),
            "skip reason must name the unmet dependency: {:?}",
            decision.skipped_specialists
        );
    }

    // ── Test 3: Invalid expression → ConditionError ─────────────────────

    #[test]
    fn test_routing_invalid_expression_returns_condition_error() {
        let fp = make_fingerprint(&[]);
        let spec = make_two_tier_spec(vec![AgentEntry {
            key: "gt-bad".into(),
            condition: Some("fingerprint.cooperation == 'non-cooperative' && &".into()),
            mandatory: false,
            depends_on: vec![],
        }]);

        let err = evaluate_routing(&spec, &fp, "run-bad", "2026-01-01T00:00:00Z").unwrap_err();
        match err {
            GameTheoryError::ConditionError {
                expression,
                message: _,
            } => {
                assert!(
                    expression.contains("fingerprint.cooperation"),
                    "error must include the offending expression"
                );
            }
            other => panic!("expected ConditionError, got {:?}", other),
        }
    }

    // ── Test 4: Cycle detection ─────────────────────────────────────────

    #[test]
    fn test_routing_cycle_detection() {
        let fp = make_fingerprint(&[]);

        // Agent A depends on B, B depends on A → cycle
        let spec = GameTheorySpec {
            version: "1.0".into(),
            spec_id: "cycle-test".into(),
            cost_cap_usd: 5.0,
            tiers: vec![
                TierEntry {
                    id: 1,
                    name: "Base".into(),
                    concurrency_cap: 4,
                    agents: vec![AgentEntry {
                        key: "gt-a".into(),
                        condition: Some("fingerprint.cooperation == 'non-cooperative'".into()),
                        mandatory: false,
                        depends_on: vec!["gt-b".into()],
                    }],
                },
                TierEntry {
                    id: 2,
                    name: "Dependent".into(),
                    concurrency_cap: 2,
                    agents: vec![AgentEntry {
                        key: "gt-b".into(),
                        condition: Some("fingerprint.payoff_sum == 'zero-sum'".into()),
                        mandatory: false,
                        depends_on: vec!["gt-a".into()],
                    }],
                },
            ],
        };

        let err = evaluate_routing(&spec, &fp, "cycle-run", "2026-01-01T00:00:00Z").unwrap_err();
        match err {
            GameTheoryError::RoutingCycle { cycle } => {
                assert!(cycle.contains("gt-a"), "cycle must mention involved agents");
                assert!(cycle.contains("gt-b"), "cycle must mention involved agents");
            }
            other => panic!("expected RoutingCycle, got {:?}", other),
        }
    }

    #[test]
    fn test_resolve_spec_path_searches_upward() {
        // Create temp dir with nested structure: root/.archon/specs/gametheory.yaml
        let temp = std::env::temp_dir().join(format!("gt-spec-test-{}", uuid::Uuid::new_v4()));
        let spec_dir = temp.join(".archon").join("specs");
        std::fs::create_dir_all(&spec_dir).unwrap();
        let spec_file = spec_dir.join("gametheory.yaml");
        std::fs::write(
            &spec_file,
            "version: \"1.0\"\nspec_id: test\ncost_cap_usd: 1.0\ntiers: []\n",
        )
        .unwrap();

        // Create nested dirs: temp/a/b/c/ (3 levels deep)
        let deep = temp.join("a").join("b").join("c");
        std::fs::create_dir_all(&deep).unwrap();

        // Run resolver from deep dir with no explicit path
        let orig_cwd = std::env::current_dir().unwrap();
        std::env::set_current_dir(&deep).unwrap();

        // Clear env var to avoid interference
        unsafe { std::env::remove_var("ARCHON_SPEC_PATH") };

        let result = resolve_spec_path(None);
        std::env::set_current_dir(&orig_cwd).unwrap();
        std::fs::remove_dir_all(&temp).unwrap();

        assert!(
            result.is_ok(),
            "should find spec via upward walk: {:?}",
            result.err()
        );
        // Compare canonicalized paths so /var ↔ /private/var symlink
        // differences on macOS don't cause spurious failures.
        let resolved = result.unwrap();
        let resolved_canonical = resolved.canonicalize().unwrap_or(resolved);
        let expected_canonical = spec_file.canonicalize().unwrap_or(spec_file);
        assert_eq!(resolved_canonical, expected_canonical);
    }
}
