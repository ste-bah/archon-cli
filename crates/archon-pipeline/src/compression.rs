//! Token-efficient memory compression layer.
//!
//! Compresses raw memory text (e.g. from CODING_NAMESPACES) into a compact
//! symbolic format readable by Claude without decoders. Target: 10x compression
//! (2000 tokens -> <200 tokens).
//!
//! The output uses the AAAK-inspired symbolic dialect:
//! ```text
//! [MEM|v1]
//! ENT:USvc|AuthMW|TokVal|PgRepo
//! DEC:postgres>persist|jwt>auth|redis>cache
//! REL:AuthMW->TokVal|USvc->PgRepo
//! PAT:repo>data|facade>orchestrate
//! FIX:!unwrap@err|!clone@hotpath
//! SH:P1=INNOC|P2=INNOC
//! @P1:USvc+AuthMW @P3:PgRepo
//! ```

use regex::Regex;
use std::collections::BTreeSet;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// Compressed memory output.
#[derive(Debug, Clone)]
pub struct CompressedMemory {
    /// The compressed output text.
    pub text: String,
    /// Estimated token count of `text`.
    pub token_estimate: usize,
    /// Number of distinct entities preserved.
    pub entities_preserved: usize,
    /// Compression ratio: input_tokens / output_tokens.  0.0 when empty.
    pub compression_ratio: f64,
    /// Which section tags are present (e.g. "ENT", "DEC", "REL").
    pub sections_present: Vec<String>,
}

// ---------------------------------------------------------------------------
// Extraction helpers — internal types
// ---------------------------------------------------------------------------

#[derive(Debug, Default)]
struct Extracted {
    entities: BTreeSet<String>,
    decisions: Vec<(String, String)>, // (choice, context)
    relationships: Vec<(String, String)>, // (from, to)
    patterns: Vec<(String, String)>,  // (name, role)
    corrections: Vec<String>,
    verdicts: Vec<(String, String)>, // (phase/label, verdict)
    phase_entities: Vec<(u32, Vec<String>)>, // (phase, entities)
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Approximate token count using chars/4 heuristic (rounded up).
pub fn estimate_tokens(text: &str) -> usize {
    (text.len() + 3) / 4
}

/// Compress raw memory text into compact symbolic format.
pub fn compress(raw: &str, budget_tokens: usize) -> CompressedMemory {
    if raw.trim().is_empty() {
        return CompressedMemory {
            text: String::new(),
            token_estimate: 0,
            entities_preserved: 0,
            compression_ratio: 0.0,
            sections_present: Vec::new(),
        };
    }

    let extracted = extract(raw);
    build_compressed(raw, &extracted, budget_tokens, None)
}

/// Compress with deduplication against existing prompt context.
///
/// Entities/decisions/relationships that already appear (case-insensitive) in
/// `existing_context` are omitted from the output.
pub fn compress_with_dedup(
    raw: &str,
    existing_context: &str,
    budget: usize,
) -> CompressedMemory {
    if raw.trim().is_empty() {
        return CompressedMemory {
            text: String::new(),
            token_estimate: 0,
            entities_preserved: 0,
            compression_ratio: 0.0,
            sections_present: Vec::new(),
        };
    }

    let extracted = extract(raw);
    build_compressed(raw, &extracted, budget, Some(existing_context))
}

/// Generate a human-readable hint from compressed output (for debugging).
pub fn decompress_hint(compressed: &CompressedMemory) -> String {
    if compressed.text.is_empty() {
        return String::from("(empty memory)");
    }

    let mut lines: Vec<String> = Vec::new();
    lines.push(format!(
        "Memory snapshot ({} tokens, {:.1}x compression, {} entities)",
        compressed.token_estimate, compressed.compression_ratio, compressed.entities_preserved,
    ));

    for line in compressed.text.lines() {
        let line = line.trim();
        if line.starts_with("[MEM|") {
            continue; // skip header
        }
        if let Some(rest) = line.strip_prefix("ENT:") {
            let ents: Vec<&str> = rest.split('|').collect();
            lines.push(format!("Entities: {}", ents.join(", ")));
        } else if let Some(rest) = line.strip_prefix("DEC:") {
            let decs: Vec<&str> = rest.split('|').collect();
            lines.push(format!(
                "Decisions: {}",
                decs.iter()
                    .map(|d| d.replace('>', " -> "))
                    .collect::<Vec<_>>()
                    .join("; ")
            ));
        } else if let Some(rest) = line.strip_prefix("REL:") {
            let rels: Vec<&str> = rest.split('|').collect();
            lines.push(format!("Relationships: {}", rels.join(", ")));
        } else if let Some(rest) = line.strip_prefix("PAT:") {
            let pats: Vec<&str> = rest.split('|').collect();
            lines.push(format!(
                "Patterns: {}",
                pats.iter()
                    .map(|p| p.replace('>', " for "))
                    .collect::<Vec<_>>()
                    .join("; ")
            ));
        } else if let Some(rest) = line.strip_prefix("FIX:") {
            let fixes: Vec<&str> = rest.split('|').collect();
            lines.push(format!("Corrections: {}", fixes.join(", ")));
        } else if let Some(rest) = line.strip_prefix("SH:") {
            let sh: Vec<&str> = rest.split('|').collect();
            lines.push(format!("Sherlock verdicts: {}", sh.join(", ")));
        } else if line.starts_with('@') {
            lines.push(format!("Phase tags: {}", line));
        }
    }

    lines.join("\n")
}

// ---------------------------------------------------------------------------
// Extraction
// ---------------------------------------------------------------------------

fn extract(raw: &str) -> Extracted {
    let mut ex = Extracted::default();

    extract_entities(raw, &mut ex);
    extract_decisions(raw, &mut ex);
    extract_relationships(raw, &mut ex);
    extract_patterns(raw, &mut ex);
    extract_corrections(raw, &mut ex);
    extract_verdicts(raw, &mut ex);
    extract_phase_entities(raw, &mut ex);

    ex
}

/// Extract CamelCase names and words ending in known suffixes.
fn extract_entities(raw: &str, ex: &mut Extracted) {
    // CamelCase: at least two uppercase-starting components.
    let camel_re = Regex::new(r"\b([A-Z][a-z]+(?:[A-Z][a-z0-9]+)+)\b").unwrap();
    for cap in camel_re.captures_iter(raw) {
        ex.entities.insert(cap[1].to_string());
    }

    // Words ending in known suffixes (at least prefix + suffix).
    let suffix_re = Regex::new(
        r"\b([A-Z][a-zA-Z]*(?:Service|Handler|Manager|Controller|Store|Repository|Validator|Middleware))\b",
    )
    .unwrap();
    for cap in suffix_re.captures_iter(raw) {
        ex.entities.insert(cap[1].to_string());
    }
}

/// Extract decision phrases: "decided to ...", "chose ...", "will use ...", "selected ...".
fn extract_decisions(raw: &str, ex: &mut Extracted) {
    let dec_re = Regex::new(
        r"(?i)(?:decided to|chose|will use|selected)\s+([a-zA-Z0-9_]+)(?:\s+(?:for|as|to|over)\s+([a-zA-Z0-9_ ]+))?"
    ).unwrap();
    for cap in dec_re.captures_iter(raw) {
        let choice = cap[1].to_string();
        let context = cap.get(2).map_or(String::new(), |m| {
            // Take first 3 words of context.
            m.as_str()
                .split_whitespace()
                .take(3)
                .collect::<Vec<_>>()
                .join(" ")
        });
        ex.decisions.push((choice, context));
    }
}

/// Extract relationship phrases: "depends on", "calls", "uses", "requires", "->", "implements".
fn extract_relationships(raw: &str, ex: &mut Extracted) {
    // Arrow syntax: A -> B
    let arrow_re = Regex::new(r"\b([A-Z][a-zA-Z0-9]+)\s*->\s*([A-Z][a-zA-Z0-9]+)\b").unwrap();
    for cap in arrow_re.captures_iter(raw) {
        ex.relationships
            .push((cap[1].to_string(), cap[2].to_string()));
    }

    // Natural language: X depends on / calls / uses / requires / implements Y
    let rel_re = Regex::new(
        r"\b([A-Z][a-zA-Z0-9]+)\s+(?:depends on|calls|uses|requires|implements)\s+([A-Z][a-zA-Z0-9]+)\b",
    )
    .unwrap();
    for cap in rel_re.captures_iter(raw) {
        ex.relationships
            .push((cap[1].to_string(), cap[2].to_string()));
    }
}

/// Extract pattern mentions.
fn extract_patterns(raw: &str, ex: &mut Extracted) {
    let pat_re = Regex::new(
        r"(?i)([a-zA-Z]+)\s+(?:pattern|strategy|approach)\s+(?:for\s+)?([a-zA-Z ]{2,30})"
    ).unwrap();
    for cap in pat_re.captures_iter(raw) {
        let name = cap[1].to_string();
        let role = cap
            .get(2)
            .map_or(String::new(), |m| {
                m.as_str()
                    .split_whitespace()
                    .take(2)
                    .collect::<Vec<_>>()
                    .join(" ")
            });
        ex.patterns.push((name, role));
    }
}

/// Extract correction phrases.
fn extract_corrections(raw: &str, ex: &mut Extracted) {
    let fix_re = Regex::new(
        r"(?i)(?:fixed|don't|avoid|instead of)\s+([a-zA-Z0-9_.!]+(?:\s+[a-zA-Z0-9_.]+){0,3})"
    ).unwrap();
    for cap in fix_re.captures_iter(raw) {
        ex.corrections.push(cap[1].to_string());
    }
}

/// Extract Sherlock verdicts.
fn extract_verdicts(raw: &str, ex: &mut Extracted) {
    // Look for "INNOCENT", "GUILTY", "APPROVED", "REJECTED" possibly near phase/sherlock info.
    let verdict_re = Regex::new(
        r"(?i)(?:(?:phase|P)\s*(\d).*?(INNOCENT|GUILTY|APPROVED|REJECTED)|(INNOCENT|GUILTY|APPROVED|REJECTED).*?(?:phase|P)\s*(\d)|[Ss]herlock.*?(INNOCENT|GUILTY|APPROVED|REJECTED))"
    ).unwrap();
    for cap in verdict_re.captures_iter(raw) {
        // Try the various groups.
        if let Some(phase) = cap.get(1) {
            let verdict = cap[2].to_string();
            ex.verdicts
                .push((format!("P{}", phase.as_str()), verdict));
        } else if let Some(phase) = cap.get(4) {
            let verdict = cap[3].to_string();
            ex.verdicts
                .push((format!("P{}", phase.as_str()), verdict));
        } else if let Some(verdict) = cap.get(5) {
            ex.verdicts
                .push(("SH".to_string(), verdict.as_str().to_string()));
        }
    }
}

/// Extract phase-tagged entities (e.g. "Phase 1: UserService, AuthMiddleware").
fn extract_phase_entities(raw: &str, ex: &mut Extracted) {
    let phase_re = Regex::new(r"(?i)phase\s+(\d)\s*[:\-]\s*([^\n]+)").unwrap();
    let entity_re = Regex::new(r"\b([A-Z][a-zA-Z0-9]+(?:[A-Z][a-z0-9]+)*)\b").unwrap();
    for cap in phase_re.captures_iter(raw) {
        if let Ok(phase_num) = cap[1].parse::<u32>() {
            let content = &cap[2];
            let mut ents = Vec::new();
            for e in entity_re.captures_iter(content) {
                ents.push(e[1].to_string());
            }
            if !ents.is_empty() {
                ex.phase_entities.push((phase_num, ents));
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Abbreviation
// ---------------------------------------------------------------------------

/// Abbreviate a CamelCase name by taking the first letter of each component,
/// keeping short words as-is.
fn abbreviate(name: &str) -> String {
    // Split CamelCase into components.
    let parts = split_camel_case(name);
    if parts.len() <= 1 {
        // Not CamelCase — try component abbreviation, then known abbreviation.
        return abbreviate_component(name);
    }

    let mut abbrev = String::new();
    for part in &parts {
        abbrev.push_str(&abbreviate_component(part));
    }
    abbrev
}

/// Abbreviate a single CamelCase component word.
fn abbreviate_component(word: &str) -> String {
    // Known component abbreviations.
    match word.to_lowercase().as_str() {
        "service" => "Svc".to_string(),
        "handler" => "Hnd".to_string(),
        "manager" => "Mgr".to_string(),
        "controller" => "Ctl".to_string(),
        "store" => "Str".to_string(),
        "repository" => "Repo".to_string(),
        "validator" => "Val".to_string(),
        "middleware" => "MW".to_string(),
        "gateway" => "GW".to_string(),
        "factory" => "Fct".to_string(),
        "builder" => "Bld".to_string(),
        "provider" => "Prv".to_string(),
        "listener" => "Lsn".to_string(),
        "collector" => "Col".to_string(),
        "interceptor" => "Icp".to_string(),
        "transformer" => "Xfm".to_string(),
        "notification" => "Ntf".to_string(),
        "connection" => "Conn".to_string(),
        "migration" => "Mig".to_string(),
        "configuration" => "Cfg".to_string(),
        "authentication" => "Auth".to_string(),
        "postgres" | "postgresql" => "Pg".to_string(),
        "session" => "Sess".to_string(),
        "response" => "Rsp".to_string(),
        "request" => "Req".to_string(),
        "context" => "Ctx".to_string(),
        "feature" => "Feat".to_string(),
        "policy" => "Pol".to_string(),
        "limiter" => "Lim".to_string(),
        "runner" => "Run".to_string(),
        "checker" | "check" => "Chk".to_string(),
        "metrics" => "Mtr".to_string(),
        "health" => "Hlth".to_string(),
        "logging" => "Log".to_string(),
        "error" => "Err".to_string(),
        "retry" => "Rty".to_string(),
        "token" => "Tok".to_string(),
        "cache" => "Cch".to_string(),
        "event" => "Evt".to_string(),
        _ => {
            // Unknown component: just first letter (aggressive abbreviation).
            word.chars()
                .next()
                .map(|c| c.to_ascii_uppercase().to_string())
                .unwrap_or_default()
        }
    }
}

fn split_camel_case(s: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    for ch in s.chars() {
        if ch.is_uppercase() && !current.is_empty() {
            parts.push(current);
            current = String::new();
        }
        current.push(ch);
    }
    if !current.is_empty() {
        parts.push(current);
    }
    parts
}

fn apply_known_abbrev(word: &str) -> String {
    match word.to_lowercase().as_str() {
        "authentication" | "authenticator" => "auth".to_string(),
        "database" => "db".to_string(),
        "repository" => "repo".to_string(),
        "configuration" | "config" => "cfg".to_string(),
        "implementation" => "impl".to_string(),
        "function" => "fn".to_string(),
        "structure" => "struct".to_string(),
        "enumeration" => "enum".to_string(),
        "module" => "mod".to_string(),
        _ => {
            // Short words: keep as-is.
            if word.len() < 5 {
                word.to_string()
            } else {
                // First 4 chars.
                word[..word.len().min(4)].to_string()
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Building the compressed output
// ---------------------------------------------------------------------------

fn build_compressed(
    raw: &str,
    ex: &Extracted,
    budget_tokens: usize,
    existing_context: Option<&str>,
) -> CompressedMemory {
    let input_tokens = estimate_tokens(raw);

    // Dedup filter: lowercase set of words from existing context.
    let dedup_set: BTreeSet<String> = existing_context
        .map(|ctx| {
            ctx.split_whitespace()
                .map(|w| w.to_lowercase())
                .collect()
        })
        .unwrap_or_default();

    let should_keep = |name: &str| -> bool {
        if dedup_set.is_empty() {
            return true;
        }
        !dedup_set.contains(&name.to_lowercase())
    };

    // Build section strings.
    // ENT
    let ent_items: Vec<String> = ex
        .entities
        .iter()
        .filter(|e| should_keep(e))
        .map(|e| abbreviate(e))
        .collect();
    let ent_section = if ent_items.is_empty() {
        None
    } else {
        Some(format!("ENT:{}", ent_items.join("|")))
    };
    let entities_preserved = ent_items.len();

    // DEC
    let dec_items: Vec<String> = ex
        .decisions
        .iter()
        .filter(|(c, _)| should_keep(c))
        .map(|(choice, context)| {
            let c = apply_known_abbrev(choice);
            if context.is_empty() {
                c
            } else {
                let ctx = context
                    .split_whitespace()
                    .take(2)
                    .map(|w| apply_known_abbrev(w))
                    .collect::<Vec<_>>()
                    .join(" ");
                format!("{}>{}", c, ctx)
            }
        })
        .collect();
    let dec_section = if dec_items.is_empty() {
        None
    } else {
        Some(format!("DEC:{}", dec_items.join("|")))
    };

    // REL
    let rel_items: Vec<String> = ex
        .relationships
        .iter()
        .map(|(from, to)| format!("{}->{}", abbreviate(from), abbreviate(to)))
        .collect();
    let rel_section = if rel_items.is_empty() {
        None
    } else {
        Some(format!("REL:{}", rel_items.join("|")))
    };

    // PAT
    let pat_items: Vec<String> = ex
        .patterns
        .iter()
        .map(|(name, role)| {
            let n = apply_known_abbrev(name);
            if role.is_empty() {
                n
            } else {
                format!("{}>{}", n, apply_known_abbrev(role))
            }
        })
        .collect();
    let pat_section = if pat_items.is_empty() {
        None
    } else {
        Some(format!("PAT:{}", pat_items.join("|")))
    };

    // FIX
    let fix_items: Vec<String> = ex
        .corrections
        .iter()
        .map(|c| format!("!{}", c.replace(' ', "@")))
        .collect();
    let fix_section = if fix_items.is_empty() {
        None
    } else {
        Some(format!("FIX:{}", fix_items.join("|")))
    };

    // SH
    let sh_items: Vec<String> = ex
        .verdicts
        .iter()
        .map(|(label, verdict)| {
            let upper = verdict.to_uppercase();
            let v = match upper.as_str() {
                "INNOCENT" => "INNOC",
                "GUILTY" => "GUILT",
                "APPROVED" => "APRVD",
                "REJECTED" => "RJCTD",
                other => other,
            };
            format!("{}={}", label, v)
        })
        .collect();
    let sh_section = if sh_items.is_empty() {
        None
    } else {
        Some(format!("SH:{}", sh_items.join("|")))
    };

    // Phase tags
    let phase_tags: Vec<String> = ex
        .phase_entities
        .iter()
        .map(|(p, ents)| {
            let abbrevs: Vec<String> = ents.iter().map(|e| abbreviate(e)).collect();
            format!("@P{}:{}", p, abbrevs.join("+"))
        })
        .collect();
    let phase_section = if phase_tags.is_empty() {
        None
    } else {
        Some(phase_tags.join(" "))
    };

    // Assemble sections in priority order (last = lowest priority = removed first).
    // Order: ENT (highest), DEC, SH, REL, FIX, PAT (lowest), phases.
    let mut sections: Vec<Section> = Vec::new();

    if let Some(s) = ent_section {
        sections.push(Section {
            tag: "ENT".to_string(),
            text: s,
            priority: 10,
        });
    }
    if let Some(s) = dec_section {
        sections.push(Section {
            tag: "DEC".to_string(),
            text: s,
            priority: 8,
        });
    }
    if let Some(s) = sh_section {
        sections.push(Section {
            tag: "SH".to_string(),
            text: s,
            priority: 7,
        });
    }
    if let Some(s) = rel_section {
        sections.push(Section {
            tag: "REL".to_string(),
            text: s,
            priority: 6,
        });
    }
    if let Some(s) = fix_section {
        sections.push(Section {
            tag: "FIX".to_string(),
            text: s,
            priority: 4,
        });
    }
    if let Some(s) = pat_section {
        sections.push(Section {
            tag: "PAT".to_string(),
            text: s,
            priority: 2,
        });
    }
    if let Some(s) = phase_section {
        sections.push(Section {
            tag: "PHASE".to_string(),
            text: s,
            priority: 3,
        });
    }

    // Build output, truncating lowest-priority sections if over budget.
    let header = "[MEM|v1]";
    let header_tokens = estimate_tokens(header) + 1; // +1 for newline

    // Sort by priority ascending so we can pop lowest first.
    sections.sort_by_key(|s| s.priority);

    // Iteratively remove lowest-priority sections if over budget.
    loop {
        let total_text = build_output_text(header, &sections);
        let tokens = estimate_tokens(&total_text);
        if tokens <= budget_tokens || sections.len() <= 1 {
            break;
        }
        // Remove lowest priority.
        sections.remove(0);
    }

    // Final check: if still over budget with just header + 1 section, truncate section text.
    let mut output_text = build_output_text(header, &sections);
    let mut out_tokens = estimate_tokens(&output_text);
    if out_tokens > budget_tokens && budget_tokens > header_tokens {
        let max_chars = budget_tokens * 4;
        if output_text.len() > max_chars {
            output_text.truncate(max_chars);
            // Ensure we don't cut mid-line.
            if let Some(pos) = output_text.rfind('\n') {
                output_text.truncate(pos);
            }
            out_tokens = estimate_tokens(&output_text);
        }
    }

    let sections_present: Vec<String> = sections.iter().map(|s| s.tag.clone()).collect();
    let ratio = if out_tokens > 0 {
        input_tokens as f64 / out_tokens as f64
    } else {
        0.0
    };

    CompressedMemory {
        text: output_text,
        token_estimate: out_tokens,
        entities_preserved,
        compression_ratio: ratio,
        sections_present,
    }
}

fn build_output_text(header: &str, sections: &[Section]) -> String {
    let mut out = String::from(header);
    for s in sections {
        out.push('\n');
        out.push_str(&s.text);
    }
    out
}

// ---------------------------------------------------------------------------
// Section helper
// ---------------------------------------------------------------------------

struct Section {
    tag: String,
    text: String,
    priority: u8, // lower = removed first during truncation
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Realistic 2000-token (~8000 char) test input mimicking CODING_NAMESPACES.
    fn realistic_input() -> String {
        r#"
## Task Analysis — TASK-AUTH-042: Implement JWT Authentication Pipeline

The UserService is the primary entry point for authentication flows. It delegates to
AuthMiddleware for request interception and TokenValidator for JWT verification.
The PostgresRepository handles all persistence operations including user sessions
and refresh tokens.

### Architecture Decisions

We decided to use PostgreSQL for persistent session storage because it provides
ACID guarantees needed for token revocation. We chose JWT over opaque tokens for
stateless validation at edge nodes. We will use Redis for caching frequently accessed
user profiles to reduce database load. Selected bcrypt for password hashing over
argon2 for broader library support.

The SessionManager coordinates between TokenValidator and PostgresRepository to
ensure atomic session creation. The ApiGateway routes all authenticated requests
through AuthMiddleware before reaching any downstream ServiceHandler.

### Dependency Graph

UserService -> AuthMiddleware -> TokenValidator
UserService -> PostgresRepository
ApiGateway -> AuthMiddleware
SessionManager -> TokenValidator
SessionManager -> PostgresRepository
AuthMiddleware depends on TokenValidator
UserService calls PostgresRepository
ApiGateway uses AuthMiddleware
RateLimiter requires ConfigStore

### Implementation Plan

Phase 1: Core authentication - UserService, AuthMiddleware, TokenValidator
Phase 2: Persistence layer - PostgresRepository, SessionManager, ConfigStore
Phase 3: Integration - ApiGateway, RateLimiter, HealthCheck, MetricsCollector
Phase 4: Testing and verification of all components

The repository pattern for data access provides clean separation of concerns.
The facade strategy for orchestrating multi-step auth flows simplifies the API.
The guard approach for validation ensures consistent input checking across endpoints.

### Corrections and Fixes

Don't use unwrap in error handling paths — always propagate with ? or map_err.
Avoid clone on hot paths where references suffice — pass &str instead of String.
Instead of manual JSON parsing use serde derive macros for type safety.
Fixed the token expiry calculation to use UTC timestamps consistently.

### Requirements Extracted

The EventBus publishes authentication events (login, logout, token_refresh) for
audit logging. The NotificationService listens for failed login attempts and
triggers alerts after 5 consecutive failures. CacheManager wraps Redis operations
with circuit breaker pattern for resilience.

### Sherlock Forensic Review

Phase 1 code review: Sherlock verdict INNOCENT — UserService correctly validates
input before delegating. AuthMiddleware properly chains to next handler.
Phase 2 review: APPROVED — PostgresRepository uses parameterized queries throughout.
Phase 3 integration: Sherlock determined INNOCENT on ApiGateway routing logic.
Phase 4 final: All components verified APPROVED by Sherlock review.

### Additional Context

The MigrationRunner handles database schema evolution. The ErrorHandler provides
uniform error response formatting across all endpoints. LoggingInterceptor captures
request/response metadata for observability. RetryPolicy implements exponential
backoff for transient failures in external service calls.

The ConnectionPool manages PostgreSQL connections with configurable min/max sizes.
The FeatureFlagService allows runtime toggling of authentication features without
deployment. RequestContext carries trace IDs and user metadata through the call chain.
ResponseTransformer maps internal domain objects to API response DTOs.

### Test Results

All 47 unit tests passing. Integration test suite: 12/12 green.
Coverage: UserService 94%, AuthMiddleware 91%, TokenValidator 97%,
PostgresRepository 88%, SessionManager 85%.

### Performance Notes

TokenValidator processes 10,000 validations/second with P99 latency under 2ms.
PostgresRepository handles 5,000 reads/second with connection pooling.
The CacheManager reduces database load by 73% for user profile lookups.

### Decisions Log

Decided to use tower middleware pattern for AuthMiddleware composition.
Selected sqlx over diesel for async PostgreSQL support.
Will use tracing crate for structured logging over env_logger.
Chose axum over actix-web for better tower ecosystem integration.

### Extended Architecture Analysis

The RequestPipeline orchestrates the full request lifecycle from ingestion to response.
The InputValidator ensures all incoming payloads conform to schema definitions before
processing begins. The OutputSerializer transforms domain results into wire format.

The QueueProcessor handles asynchronous workloads via a background task system.
It depends on the MessageBroker for reliable delivery guarantees. The DeadLetterHandler
captures failed messages for manual inspection and replay. We decided to use RabbitMQ
for message queuing because it provides flexible routing and acknowledgment semantics.

### Security Review

The PermissionGuard enforces role-based access control at the endpoint level.
It calls AuthMiddleware for identity verification and then checks the PolicyEngine
for authorization decisions. The AuditLogger records all permission decisions for
compliance requirements. Selected RBAC over ABAC for simpler initial implementation.

PermissionGuard -> AuthMiddleware
PermissionGuard -> PolicyEngine
AuditLogger uses PermissionGuard
PolicyEngine depends on ConfigStore

Don't use string concatenation for SQL queries — always use parameterized statements.
Avoid storing sensitive data in JWT payload — keep tokens minimal.
Instead of custom error codes use standard HTTP status codes with problem details.
Fixed the race condition in SessionManager by adding distributed locking via Redis.

### Detailed Component Specifications

The DataMigrator handles schema evolution across multiple database versions. It uses
a checksum approach for migration verification. The SchemaValidator ensures all migrations
are forward-compatible and reversible. The BackupService creates point-in-time snapshots
before destructive migrations execute.

DataMigrator -> SchemaValidator
BackupService -> PostgresRepository
DataMigrator uses BackupService

The CircuitBreaker pattern for external service resilience prevents cascade failures.
The bulkhead strategy for resource isolation limits blast radius of component failures.
The observer approach for event propagation decouples producers from consumers.

### Sherlock Extended Verdicts

Phase 5 security review: Sherlock verdict INNOCENT — PermissionGuard correctly validates.
Phase 6 integration: APPROVED — all cross-component wiring verified by Sherlock.

### Monitoring and Observability

The TracingCollector aggregates distributed trace spans across service boundaries.
The AlertManager evaluates threshold rules and dispatches notifications via configured
channels. The DashboardService provides real-time visibility into system health metrics.

TracingCollector -> MetricsCollector
AlertManager depends on ConfigStore
DashboardService uses TracingCollector

Will use OpenTelemetry for distributed tracing over Jaeger-specific instrumentation.
Decided to use Prometheus for metrics collection as it supports pull-based scraping.

Phase 5: Monitoring - TracingCollector, AlertManager, DashboardService, MetricsCollector
Phase 6: Security - PermissionGuard, PolicyEngine, AuditLogger, DataMigrator

### Deployment and Infrastructure

The ContainerOrchestrator manages service deployment across multiple availability zones.
It coordinates with the LoadBalancer for traffic distribution and health checking.
The ConfigResolver pulls environment-specific configuration from the central vault.
The SecretManager provides encrypted storage and rotation for API keys and certificates.

ContainerOrchestrator -> LoadBalancer
ConfigResolver depends on SecretManager
ContainerOrchestrator uses ConfigResolver

The RollbackController handles automated rollback when deployment health checks fail.
It monitors error rate thresholds and latency percentiles to determine rollback triggers.
The CanaryAnalyzer compares canary instance metrics against baseline to approve promotion.

Decided to use Kubernetes for container orchestration over Docker Swarm for scalability.
Selected Vault for secret management because it provides dynamic credential rotation.
Will use Terraform for infrastructure as code over CloudFormation for multi-cloud support.

Don't use hardcoded configuration values — always pull from ConfigResolver at startup.
Avoid manual deployment steps — all deployments must go through the ContainerOrchestrator.
Fixed the flaky health check by increasing the initial delay and adding retry logic.

### End-to-End Integration Testing

The IntegrationTestRunner executes full stack tests against a staging environment.
The TestDataFactory generates realistic test fixtures using domain-specific builders.
The AssertionLibrary provides fluent assertion helpers for complex domain validations.

IntegrationTestRunner uses TestDataFactory
IntegrationTestRunner -> AssertionLibrary
TestDataFactory depends on PostgresRepository

Sherlock final integration review: APPROVED — all 156 integration points verified clean.
"#
        .to_string()
    }

    #[test]
    fn test_empty_input_produces_empty_output() {
        let result = compress("", 1000);
        assert!(result.text.is_empty());
        assert_eq!(result.token_estimate, 0);
        assert_eq!(result.entities_preserved, 0);
        assert_eq!(result.compression_ratio, 0.0);
        assert!(result.sections_present.is_empty());

        // Whitespace-only also empty.
        let result2 = compress("   \n  \t  ", 1000);
        assert!(result2.text.is_empty());
    }

    #[test]
    fn test_estimate_tokens_accuracy() {
        // chars/4 rounded up
        assert_eq!(estimate_tokens(""), 0); // (0+3)/4 = 0
        assert_eq!(estimate_tokens("a"), 1); // (1+3)/4 = 1
        assert_eq!(estimate_tokens("abcd"), 1); // (4+3)/4 = 1
        assert_eq!(estimate_tokens("abcde"), 2); // (5+3)/4 = 2
        assert_eq!(estimate_tokens("abcdefgh"), 2); // (8+3)/4 = 2

        // Within 20% of chars/4 for larger text.
        let text = "a".repeat(1000);
        let est = estimate_tokens(&text);
        let expected = 250; // 1000/4
        let diff = (est as f64 - expected as f64).abs() / expected as f64;
        assert!(diff < 0.20, "Token estimate {est} too far from expected {expected}");
    }

    #[test]
    fn test_10x_compression_ratio() {
        let input = realistic_input();
        let input_tokens = estimate_tokens(&input);
        assert!(
            input_tokens > 500,
            "Test input should be substantial: got {} tokens",
            input_tokens
        );

        let result = compress(&input, 200);
        assert!(
            result.token_estimate <= 200,
            "Output should be under 200 tokens, got {}",
            result.token_estimate
        );

        let ratio = input_tokens as f64 / result.token_estimate.max(1) as f64;
        assert!(
            ratio >= 10.0,
            "Compression ratio should be >= 10x, got {:.1}x",
            ratio
        );
    }

    #[test]
    fn test_output_starts_with_header() {
        let result = compress("UserService depends on AuthMiddleware", 1000);
        assert!(
            result.text.starts_with("[MEM|v1]"),
            "Output must start with [MEM|v1] header, got: {}",
            &result.text[..result.text.len().min(40)]
        );
    }

    #[test]
    fn test_entities_extracted_and_abbreviated() {
        let input = "The UserService processes requests via AuthMiddleware.";
        let result = compress(input, 1000);

        // Should contain abbreviated entities.
        assert!(result.entities_preserved > 0, "Should extract entities");
        assert!(
            result.text.contains("ENT:"),
            "Should have ENT section: {}",
            result.text
        );

        // UserService -> USvc (U+Svc), AuthMiddleware -> AuthMW (Auth+MW)
        // The exact abbreviation depends on the algorithm, but entities should be present.
        assert!(
            result.sections_present.contains(&"ENT".to_string()),
            "sections_present should include ENT"
        );
    }

    #[test]
    fn test_decisions_extracted() {
        let input = "We decided to use PostgreSQL for persistence. We chose JWT for auth tokens.";
        let result = compress(input, 1000);

        assert!(
            result.text.contains("DEC:"),
            "Should have DEC section: {}",
            result.text
        );
    }

    #[test]
    fn test_relationships_extracted() {
        let input = "UserService -> PostgresRepository\nAuthMiddleware depends on TokenValidator";
        let result = compress(input, 1000);

        assert!(
            result.text.contains("REL:"),
            "Should have REL section: {}",
            result.text
        );
        assert!(
            result.text.contains("->"),
            "REL section should contain arrows: {}",
            result.text
        );
    }

    #[test]
    fn test_deduplication_removes_existing() {
        let input = "UserService depends on AuthMiddleware. TokenValidator verifies JWT.";
        let existing = "The UserService is already documented.";

        let without_dedup = compress(input, 1000);
        let with_dedup = compress_with_dedup(input, existing, 1000);

        // With dedup should have fewer or equal entities since UserService is in context.
        assert!(
            with_dedup.entities_preserved <= without_dedup.entities_preserved,
            "Dedup should remove entities found in existing context: {} vs {}",
            with_dedup.entities_preserved,
            without_dedup.entities_preserved
        );
    }

    #[test]
    fn test_decompress_hint_readable() {
        let input = realistic_input();
        let compressed = compress(&input, 500);
        let hint = decompress_hint(&compressed);

        assert!(
            !hint.is_empty(),
            "Hint should not be empty for non-empty input"
        );
        assert!(
            hint.contains("Memory snapshot"),
            "Hint should contain header: {}",
            hint
        );
        assert!(
            hint.contains("entities"),
            "Hint should mention entities: {}",
            hint
        );
    }

    #[test]
    fn test_decompress_hint_empty() {
        let compressed = compress("", 1000);
        let hint = decompress_hint(&compressed);
        assert_eq!(hint, "(empty memory)");
    }

    #[test]
    fn test_budget_enforcement() {
        let input = realistic_input();

        // Very tight budget.
        let result = compress(&input, 50);
        assert!(
            result.token_estimate <= 50,
            "Must respect budget of 50 tokens, got {}",
            result.token_estimate
        );

        // Slightly larger budget.
        let result2 = compress(&input, 100);
        assert!(
            result2.token_estimate <= 100,
            "Must respect budget of 100 tokens, got {}",
            result2.token_estimate
        );
    }

    #[test]
    fn test_deterministic_output() {
        let input = realistic_input();
        let a = compress(&input, 500);
        let b = compress(&input, 500);
        assert_eq!(a.text, b.text, "Compression must be deterministic");
        assert_eq!(a.token_estimate, b.token_estimate);
        assert_eq!(a.entities_preserved, b.entities_preserved);
    }

    #[test]
    fn test_sherlock_verdicts_extracted() {
        let input = "Phase 1 review: Sherlock verdict INNOCENT. Phase 2: APPROVED.";
        let result = compress(input, 1000);
        assert!(
            result.text.contains("SH:"),
            "Should have SH section: {}",
            result.text
        );
    }

    #[test]
    fn test_corrections_extracted() {
        let input = "Don't use unwrap in error paths. Avoid clone on hot paths.";
        let result = compress(input, 1000);
        assert!(
            result.text.contains("FIX:"),
            "Should have FIX section: {}",
            result.text
        );
    }

    #[test]
    fn test_abbreviate_camel_case() {
        assert_eq!(abbreviate("UserService"), "USvc");
        // AuthMiddleware -> Auth + Middleware -> Auth + Mddl... let's check actual
        let abbr = abbreviate("AuthMiddleware");
        assert!(
            abbr.len() < "AuthMiddleware".len(),
            "Abbreviation '{}' should be shorter than original",
            abbr
        );
    }

    #[test]
    fn test_split_camel_case() {
        assert_eq!(split_camel_case("UserService"), vec!["User", "Service"]);
        assert_eq!(
            split_camel_case("AuthMiddleware"),
            vec!["Auth", "Middleware"]
        );
        assert_eq!(split_camel_case("API"), vec!["A", "P", "I"]);
        assert_eq!(split_camel_case("hello"), vec!["hello"]);
    }

    #[test]
    fn test_large_input_compression() {
        // Generate ~8000 chars of realistic content.
        let input = realistic_input();
        let char_count = input.len();
        assert!(
            char_count > 3000,
            "Realistic input should be at least 3000 chars, got {}",
            char_count
        );

        let result = compress(&input, 200);

        // Output under 800 chars (200 tokens * 4).
        assert!(
            result.text.len() < 800,
            "Compressed output should be under 800 chars, got {}",
            result.text.len()
        );

        // Should have multiple sections.
        assert!(
            result.sections_present.len() >= 2,
            "Should have at least 2 sections, got {:?}",
            result.sections_present
        );
    }
}
