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
        assert!(
            diff < 0.20,
            "Token estimate {est} too far from expected {expected}"
        );
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
