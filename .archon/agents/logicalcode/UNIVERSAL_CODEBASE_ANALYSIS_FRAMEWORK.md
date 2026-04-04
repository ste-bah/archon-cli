---
name: universal-codebase-analysis-framework
description: Universal codebase analysis framework for systematic deep logical inconsistency analysis on any codebase. Combines prompt engineering best practices with comprehensive code review methodologies to identify subtle bugs and architectural flaws.
---

# Universal Codebase Analysis Framework
## Dynamic Deep Analysis System for Logical Inconsistencies

---

## Framework Overview

This universal framework provides a systematic, repeatable approach to conducting deep logical inconsistency analysis on any codebase. It combines best practices in prompt engineering with comprehensive code review methodologies to identify subtle bugs, architectural flaws, and business logic errors that pass syntax validation but cause runtime failures or incorrect behavior.

---

## Part 1: Configuration Module

### 1.1 Project Context Configuration

**Fill out this section before beginning analysis:**

```yaml
PROJECT_PROFILE:
  name: "[Project Name]"
  version: "[Version/Release]"
  analysis_date: "[Date]"
  
TECHNICAL_STACK:
  primary_language: "[e.g., Python, JavaScript, Java, C#, Go, Rust]"
  language_version: "[e.g., Python 3.11, ES2022, Java 17]"
  frameworks: "[e.g., React 18, Django 4.2, Spring Boot 3.0]"
  key_libraries: "[e.g., Redux, Celery, Hibernate, gRPC]"
  runtime_environment: "[e.g., Node.js 20, .NET 8, JVM]"
  
ARCHITECTURE:
  pattern: "[e.g., microservices, monolithic, event-driven, serverless, hexagonal]"
  data_layer: "[e.g., PostgreSQL, MongoDB, Redis, event sourcing]"
  communication: "[e.g., REST, GraphQL, gRPC, message queues]"
  deployment: "[e.g., Kubernetes, Docker, serverless functions]"
  
BUSINESS_DOMAIN:
  industry: "[e.g., fintech, healthcare, e-commerce, SaaS, IoT]"
  core_functionality: "[Brief description of what the system does]"
  user_base_scale: "[e.g., B2C millions of users, B2B enterprise, internal tool]"
  
CRITICAL_CONSTRAINTS:
  regulatory: "[e.g., HIPAA, GDPR, PCI-DSS, SOC2, none]"
  performance: "[e.g., <100ms response time, real-time processing, batch processing]"
  reliability: "[e.g., 99.99% uptime, eventual consistency acceptable]"
  security: "[e.g., financial data, PII handling, public-facing API]"
  scalability: "[e.g., must handle 1M+ requests/day, horizontal scaling required]"

ANALYSIS_SCOPE:
  focus_areas: "[e.g., authentication module, payment processing, data pipeline]"
  exclusions: "[e.g., third-party libraries, generated code, deprecated modules]"
  priority_level: "[CRITICAL / HIGH / MEDIUM / LOW - determines depth of analysis]"
```

### 1.2 Analysis Configuration

**Select analysis depth and categories:**

```yaml
ANALYSIS_DEPTH:
  mode: "[COMPREHENSIVE / TARGETED / QUICK_SCAN]"
  # COMPREHENSIVE: All categories, full depth
  # TARGETED: Selected categories, focus on high-risk areas
  # QUICK_SCAN: Surface-level review, critical issues only

ENABLED_CATEGORIES:
  control_flow: true
  data_state_management: true
  business_logic: true
  concurrency_timing: true
  error_handling: true
  architectural_consistency: true
  api_contracts: true
  security_logic: true

SEVERITY_THRESHOLD:
  report_minimum: "[CRITICAL / HIGH / MEDIUM / LOW]"
  auto_flag_patterns: "[List specific anti-patterns to auto-detect]"
```

---

## Part 2: AI Agent Prompt Generator

**This section dynamically generates the analysis prompt based on your configuration.**

### 2.1 Role and Context Injection

```markdown
ROLE AND CONTEXT:

You are a senior software architect and security auditor with 15+ years of experience in code review, debugging, and systems design. You specialize in identifying subtle logical errors, architectural inconsistencies, and business logic flaws that pass syntax validation but cause runtime failures or produce incorrect behavior.

CODEBASE CONTEXT:
- Programming Language: {{PRIMARY_LANGUAGE}} ({{LANGUAGE_VERSION}})
- Framework/Libraries: {{FRAMEWORKS}} with {{KEY_LIBRARIES}}
- Architecture Pattern: {{ARCHITECTURE_PATTERN}}
- Business Domain: {{INDUSTRY}} - {{CORE_FUNCTIONALITY}}
- Critical Constraints: {{REGULATORY}}, {{PERFORMANCE}}, {{SECURITY}}
- Analysis Scope: {{FOCUS_AREAS}}
- Excluded from Analysis: {{EXCLUSIONS}}

SYSTEM CHARACTERISTICS:
- Runtime Environment: {{RUNTIME_ENVIRONMENT}}
- Data Layer: {{DATA_LAYER}}
- Communication Patterns: {{COMMUNICATION}}
- Deployment Model: {{DEPLOYMENT}}
- Scale Requirements: {{USER_BASE_SCALE}}, {{SCALABILITY}}
- Reliability Requirements: {{RELIABILITY}}
```

### 2.2 Primary Objective

```markdown
PRIMARY OBJECTIVE:

Conduct a {{ANALYSIS_DEPTH}} analysis of the provided codebase to identify logical inconsistencies, flawed reasoning, and business logic errors. Focus on issues that are syntactically correct but produce incorrect behavior, unexpected results, or violate system invariants.

Given the {{INDUSTRY}} domain and {{CRITICAL_CONSTRAINTS}}, pay special attention to:
- Data integrity and consistency
- Security and access control logic
- Business rule violations
- Compliance with {{REGULATORY}} requirements
- Performance implications affecting {{PERFORMANCE}} targets
- Reliability issues that could impact {{RELIABILITY}} goals
```

---

## Part 3: Analysis Categories Framework

**Comprehensive breakdown of logical inconsistency types:**

### Category 1: Control Flow Logic Errors

**Detection Focus:**
- Incorrect conditional statements (using `=` instead of `==`, wrong logical operators)
- Unreachable code blocks or dead code paths
- Missing or incorrect loop termination conditions
- Off-by-one errors in iteration boundaries (fencepost errors)
- Incorrect loop variable usage or mutation
- Missing break/continue statements causing unintended fall-through
- Inverted boolean logic or double negatives
- Short-circuit evaluation issues
- Incorrect switch/case fall-through behavior
- Missing default cases in switch statements

**Language-Specific Patterns:**
```yaml
Python: 
  - "if x = 5" (assignment in condition)
  - Mutable default arguments in function definitions
  - Using 'is' instead of '==' for value comparison
JavaScript:
  - "==" vs "===" confusion causing type coercion
  - Missing 'break' in switch statements
  - Async function without await
Java/C#:
  - Missing break in switch causing fall-through
  - Assignment in if condition: if (x = 5)
  - Comparing objects with == instead of .equals()
Go:
  - Incorrect goroutine closure over loop variables
  - Missing mutex locks for shared state
Rust:
  - Ownership/borrowing logic errors
  - Unwrap() on Result without error handling
```

### Category 2: Data Type and State Management Errors

**Detection Focus:**
- Type coercion issues leading to unexpected behavior
- Incorrect data type usage for domain (e.g., floats for currency, strings for IDs)
- State mutation in unexpected contexts (especially in functional/immutable patterns)
- Shared mutable state causing race conditions or inconsistent reads
- Missing null/undefined/None checks before dereferencing
- Incorrect assumptions about data structure shape or schema
- Primitive obsession (using primitives instead of value objects for domain concepts)
- Incorrect handling of optional/nullable types
- State leakage between request contexts
- Incorrect state initialization or reset

**Domain-Specific Checks:**
```yaml
Financial Data:
  - Using float/double for currency (should use decimal/BigDecimal)
  - Incorrect rounding modes for monetary calculations
  - Missing precision specifications
Healthcare Data:
  - Date/time handling without timezone awareness
  - Incorrect handling of missing/unknown values vs zero
E-commerce:
  - Inventory counts using signed integers (allowing negative)
  - Price/quantity as float causing precision loss
  - Cart state not properly isolated between users
```

### Category 3: Business Logic Violations

**Detection Focus:**
- Calculations that violate business rules (negative prices, >100% discounts, invalid dates)
- Incorrect order of operations in multi-step business processes
- Missing validation for business constraints (age limits, geographic restrictions, time windows)
- Logic that allows invalid state transitions (order shipped before payment confirmed)
- Incorrect handling of edge cases in business workflows
- Missing or incorrect authorization checks for sensitive operations
- Revenue or data leakage through logic flaws
- Incorrect application of business rules across different user tiers/roles
- Missing idempotency for financial transactions
- Incorrect handling of partial failures in multi-step processes

**Business Rule Validation Template:**
```markdown
For each business operation, verify:
1. Pre-conditions: Are all required conditions checked before execution?
2. Invariants: Does the operation maintain system invariants?
3. Post-conditions: Are outcomes validated?
4. Atomicity: Can partial execution leave system in invalid state?
5. Authorization: Is access control properly enforced?
6. Audit: Are critical operations logged?
```

### Category 4: Concurrency and Timing Issues

**Detection Focus:**
- Race conditions in shared resource access
- Incorrect synchronization or locking mechanisms (wrong lock granularity, lock ordering)
- Deadlock-prone code patterns (circular wait conditions)
- Missing atomic operations where required
- Incorrect async/await usage or promise handling
- Time-of-check to time-of-use (TOCTOU) vulnerabilities
- Lost updates in concurrent writes
- Non-atomic read-modify-write sequences
- Incorrect use of thread-local storage
- Missing happens-before relationships

**Concurrency Pattern Analysis:**
```yaml
Async/Await Patterns:
  - Awaiting in loops (sequential instead of parallel)
  - Missing error handling in promise chains
  - Unhandled promise rejections
  - Race conditions in parallel promises
  
Locking Patterns:
  - Lock held during I/O operations
  - Missing locks for shared state
  - Incorrect lock hierarchy (deadlock risk)
  - Locks held across await/async boundaries
  
Distributed Systems:
  - Missing distributed locks for critical sections
  - Eventual consistency not properly handled
  - Missing idempotency for retried operations
  - Clock skew issues in distributed timestamps
```

### Category 5: Error Handling and Edge Cases

**Detection Focus:**
- Missing error handling for critical operations (I/O, network, database)
- Incorrect exception types being caught or thrown
- Silent failure modes that mask underlying issues (empty catch blocks)
- Inadequate validation of inputs (especially user inputs, API responses, file uploads)
- Missing boundary condition checks (empty arrays, zero values, maximum values, null)
- Incorrect assumptions about external system availability or behavior
- Resource leaks (unclosed files, database connections, memory, sockets)
- Incorrect error recovery logic
- Missing fallback behavior for degraded operations
- Swallowing errors without logging

**Edge Case Checklist:**
```markdown
For each function, verify handling of:
□ Null/undefined/None inputs
□ Empty collections (arrays, lists, maps)
□ Zero and negative numbers
□ Maximum/minimum values (integer overflow, string length)
□ Boundary values (first, last, one-before-first, one-after-last)
□ Duplicate entries
□ Concurrent modifications
□ Partially constructed objects
□ Invalid state during initialization
□ Missing required fields
□ Malformed input data
□ Network timeouts
□ Database connection failures
□ Disk full conditions
□ Memory exhaustion
```

### Category 6: Architectural and Design Inconsistencies

**Detection Focus:**
- Violations of established architectural patterns within the codebase
- Inconsistent error propagation strategies (mixing exceptions, error codes, Either/Result)
- Mixed abstraction levels within single functions
- Circular dependencies or tight coupling causing logical fragility
- Incorrect separation of concerns leading to logic duplication
- Stateful operations in supposedly stateless contexts
- Missing idempotency where required (APIs, message handlers)
- Inconsistent use of design patterns
- Violation of SOLID principles leading to logical brittleness
- Inconsistent dependency injection patterns

**Architectural Consistency Matrix:**
```yaml
Check consistency across:
  - Error handling strategy (exceptions vs Result types vs error codes)
  - Logging patterns (what/when/how to log)
  - Authentication/authorization checks placement
  - Transaction boundary management
  - Caching strategy
  - Retry and timeout configurations
  - Configuration management approach
  - Naming conventions for similar operations
```

### Category 7: API and Interface Contract Violations

**Detection Focus:**
- Function return types or values inconsistent with documentation
- Side effects in functions expected to be pure
- Mutating input parameters unexpectedly
- Breaking interface contracts or LSP (Liskov Substitution Principle)
- Inconsistent API behavior across similar endpoints
- Missing or incorrect API versioning
- Breaking changes in supposedly stable interfaces
- Incorrect HTTP status codes for error conditions
- Missing required fields in API responses
- Inconsistent error response formats
- Violating REST/GraphQL conventions

**API Contract Verification:**
```markdown
For each API endpoint/function, verify:
1. Contract: Does implementation match documented behavior?
2. Immutability: Are inputs modified unexpectedly?
3. Idempotency: Can operation be safely retried?
4. Consistency: Do similar operations behave similarly?
5. Versioning: Are breaking changes properly versioned?
6. Error Contracts: Are error responses consistent and documented?
```

### Category 8: Security Logic Flaws

**Detection Focus:**
- Authentication/authorization logic bypasses (privilege escalation, IDOR)
- SQL injection vulnerabilities through string concatenation
- Missing input sanitization leading to XSS, log injection, or command injection
- Cryptographic failures (weak algorithms, hardcoded keys, improper random generation)
- Insufficient access control checks (checking only once, not on every access)
- Logic that exposes sensitive data or internal state
- Missing CSRF protection
- Insecure deserialization
- Path traversal vulnerabilities
- Missing rate limiting on sensitive operations
- Incorrect session management
- Authentication logic that can be bypassed

**Security Checklist by Domain:**
```yaml
Financial Systems:
  - Transaction authorization at every step
  - Audit logging for all money movements
  - Multi-factor authentication for high-value operations
  - Rate limiting on transfer operations
  
Healthcare Systems:
  - PHI access logging and authorization
  - Data encryption at rest and in transit
  - Audit trails for all data access
  - Consent validation before data disclosure
  
Public APIs:
  - Rate limiting per user/IP
  - Input validation and sanitization
  - Authentication on all protected endpoints
  - SQL injection prevention (parameterized queries)
  - XSS prevention (output encoding)
```

---

## Part 4: Analysis Methodology

### 4.1 Systematic Analysis Process

**For each file and function in the codebase, execute:**

**Step 1: Initial Assessment**
```markdown
1. Identify the purpose and intended behavior
2. List all inputs, outputs, and side effects
3. Note any assumptions made by the code
4. Identify dependencies and interactions
```

**Step 2: Execution Path Tracing**
```markdown
1. Trace happy path execution
2. Identify all conditional branches
3. Trace error paths and exception handling
4. Consider edge cases and boundary conditions
5. Map all possible state transitions
```

**Step 3: Assumption Validation**
```markdown
1. List implicit assumptions about:
   - Input data format and validity
   - External system availability and behavior
   - Execution order and timing
   - State consistency
   - Resource availability
2. Verify each assumption is actually enforced
```

**Step 4: Invariant Verification**
```markdown
1. Identify system invariants that must hold
2. Trace whether code maintains invariants throughout execution
3. Check if error handling preserves invariants
4. Verify invariants at transaction boundaries
```

**Step 5: Boundary Condition Testing**
```markdown
1. Test behavior with empty inputs
2. Test behavior with null/undefined/None
3. Test behavior with zero and negative values
4. Test behavior with maximum values
5. Test behavior with malformed data
```

**Step 6: Consistency Analysis**
```markdown
1. Compare similar functions for consistent patterns
2. Check error handling consistency
3. Verify naming conventions match behavior
4. Ensure similar operations produce similar results
```

**Step 7: Interaction Analysis**
```markdown
1. Verify component contracts are honored
2. Check data flow between components
3. Analyze side effect propagation
4. Evaluate temporal dependencies
5. Check for resource sharing conflicts
```

### 4.2 Chain-of-Thought Reasoning Template

**For each potential issue identified, apply this reasoning structure:**

```markdown
ANALYSIS CHAIN FOR [Function/Module Name]:

1. STATED INTENT:
   "The code appears to intend to: [describe apparent purpose]"

2. ACTUAL EXECUTION FLOW:
   "When executed, the code will:
   - Step 1: [trace execution]
   - Step 2: [trace execution]
   - Step 3: [trace execution]"

3. DIVERGENCE POINT:
   "The intent and execution diverge at: [specific location]
   Because: [reason for divergence]"

4. IMPACT ANALYSIS:
   "This divergence is problematic because:
   - Immediate impact: [what breaks now]
   - System impact: [wider consequences]
   - User impact: [how users are affected]
   - Data impact: [data integrity concerns]"

5. ALIGNMENT SOLUTION:
   "To align execution with intent:
   - Change: [specific modification]
   - Rationale: [why this fixes the issue]
   - Verification: [how to confirm fix works]"
```

---

## Part 5: Output Format Specification

### 5.1 Issue Report Template

**For each logical inconsistency identified:**

```markdown
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

ISSUE #[ID]: [Brief Title]

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

SEVERITY: [🔴 CRITICAL | 🟠 HIGH | 🟡 MEDIUM | 🔵 LOW]

CATEGORY: [One of 8 categories]

LOCATION: 
- File: [file path]
- Function/Method: [name]
- Lines: [line numbers]
- Module: [parent module/package]

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

DESCRIPTION:
[Clear, concise explanation of the logical flaw]

WHY THIS MATTERS:
Business Impact: [effect on business operations]
User Impact: [effect on user experience]
Data Impact: [effect on data integrity]
Security Impact: [security implications if any]
Financial Impact: [cost/revenue implications if any]

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

REPRODUCTION SCENARIO:
Prerequisites: [required setup/state]
Steps:
1. [action]
2. [action]
3. [action]
Expected: [what should happen]
Actual: [what actually happens]

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

CURRENT BEHAVIOR:
[Detailed explanation of current (incorrect) behavior]

EXPECTED BEHAVIOR:
[Detailed explanation of correct behavior]

ROOT CAUSE:
[Underlying reason for the error - misunderstanding, incorrect assumption, etc.]

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

RECOMMENDED FIX:

Current Code:
```[language]
[problematic code snippet]
```

Proposed Fix:
```[language]
[corrected code snippet]
```

Explanation:
[Why this fix resolves the issue]

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

TRADE-OFFS:
Pros: [benefits of the fix]
Cons: [potential drawbacks]
Performance: [performance implications]
Compatibility: [breaking changes if any]

RELATED ISSUES: [Links to related issue IDs if any]

CONFIDENCE LEVEL: [HIGH / MEDIUM / LOW / NEEDS_CLARIFICATION]

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
```

### 5.2 Summary Report Template

```markdown
╔═══════════════════════════════════════════════════════════════╗
║         CODEBASE ANALYSIS SUMMARY REPORT                      ║
╚═══════════════════════════════════════════════════════════════╝

PROJECT: {{PROJECT_NAME}}
ANALYSIS DATE: {{ANALYSIS_DATE}}
ANALYZER: [AI Model/Human Reviewer]
SCOPE: {{FOCUS_AREAS}}

─────────────────────────────────────────────────────────────────

EXECUTIVE SUMMARY:

[High-level overview of findings and overall code health]

─────────────────────────────────────────────────────────────────

STATISTICS:

Total Issues Found: [number]
├─ 🔴 Critical: [count] ([percentage]%)
├─ 🟠 High: [count] ([percentage]%)
├─ 🟡 Medium: [count] ([percentage]%)
└─ 🔵 Low: [count] ([percentage]%)

Issues by Category:
├─ Control Flow Logic: [count]
├─ Data/State Management: [count]
├─ Business Logic: [count]
├─ Concurrency/Timing: [count]
├─ Error Handling: [count]
├─ Architectural: [count]
├─ API Contracts: [count]
└─ Security: [count]

Files Analyzed: [count]
Functions Analyzed: [count]
Lines of Code Analyzed: [count]

─────────────────────────────────────────────────────────────────

TOP PRIORITY ISSUES:

1. [Issue ID] - [Brief description] (CRITICAL)
2. [Issue ID] - [Brief description] (CRITICAL)
3. [Issue ID] - [Brief description] (HIGH)
4. [Issue ID] - [Brief description] (HIGH)
5. [Issue ID] - [Brief description] (HIGH)

─────────────────────────────────────────────────────────────────

RISK ASSESSMENT:

Immediate Risks:
- [Risk area and description]
- [Risk area and description]

Medium-term Concerns:
- [Concern and description]
- [Concern and description]

Technical Debt:
- [Debt area and description]
- [Debt area and description]

─────────────────────────────────────────────────────────────────

RECOMMENDATIONS:

Short-term (0-2 weeks):
1. [Action item]
2. [Action item]

Medium-term (1-3 months):
3. [Action item]
4. [Action item]

Long-term (3-6 months):
5. [Action item]
6. [Action item]

─────────────────────────────────────────────────────────────────

POSITIVE OBSERVATIONS:

- [Well-implemented pattern or practice]
- [Strong area of the codebase]
- [Good architectural decision]

─────────────────────────────────────────────────────────────────
```

---

## Part 6: Usage Instructions

### 6.1 Quick Start Guide

**Step 1: Configure Your Analysis**
```bash
1. Fill out the Configuration Module (Part 1)
2. Select analysis depth and enabled categories
3. Set severity threshold for reporting
```

**Step 2: Generate Analysis Prompt**
```bash
1. Use Part 2 to generate the AI prompt with your configuration
2. Replace all {{PLACEHOLDERS}} with your actual values
3. Copy the complete prompt
```

**Step 3: Prepare Your Codebase**
```bash
1. Identify the specific files/modules to analyze
2. Ensure code is properly formatted and documented
3. Include any relevant context files (README, architecture docs)
```

**Step 4: Execute Analysis**
```bash
1. Paste the generated prompt to your AI assistant
2. Provide the codebase files/modules
3. Review the output
4. Iterate with follow-up prompts for deeper analysis
```

**Step 5: Process Results**
```bash
1. Review all identified issues
2. Validate findings (AI outputs require human verification)
3. Prioritize fixes based on severity and business impact
4. Create tickets/tasks for remediation
```

### 6.2 Advanced Usage Patterns

**Pattern 1: Incremental Analysis**
```markdown
Analyze critical modules first:
1. Authentication/Authorization
2. Payment Processing
3. Data Access Layer
4. API Endpoints
5. Business Logic Core
Then expand to supporting modules.
```

**Pattern 2: Targeted Deep Dive**
```markdown
If initial scan reveals issues in specific category:
1. Re-run analysis with only that category enabled
2. Set depth to COMPREHENSIVE
3. Focus on the affected modules
4. Request specific reproduction scenarios
```

**Pattern 3: Security-Focused Audit**
```markdown
Enable only security-related categories:
- Category 8: Security Logic Flaws
- Category 3: Business Logic (for authorization)
- Category 5: Error Handling (for information disclosure)
Set severity threshold to MEDIUM
Request OWASP Top 10 mapping for each issue
```

**Pattern 4: Performance Impact Analysis**
```markdown
Enable concurrency and architectural categories:
- Category 4: Concurrency/Timing
- Category 6: Architectural Consistency
Request performance profiling implications
Focus on hot paths and high-traffic endpoints
```

### 6.3 Iteration Strategies

**Follow-up Prompt Templates:**

**For Deeper Analysis:**
```markdown
"Based on Issue #[ID], perform a deeper analysis of the [module name] focusing specifically on [aspect]. Trace all execution paths and consider [specific edge case]."
```

**For Context Clarification:**
```markdown
"Issue #[ID] seems concerning, but I need more context. The [pattern] is used throughout our codebase because [reason]. Re-evaluate this issue considering this architectural decision."
```

**For Fix Validation:**
```markdown
"Here is my proposed fix for Issue #[ID]: [code]. Analyze this fix for:
1. Correctness - does it solve the problem?
2. Completeness - are there edge cases missed?
3. Side effects - does it introduce new issues?
4. Performance - what is the performance impact?"
```

**For Related Issue Discovery:**
```markdown
"Issue #[ID] suggests a pattern problem. Search the entire codebase for similar patterns to [specific pattern] and report all instances."
```

---

## Part 7: Quality Assurance Guidelines

### 7.1 Validation Checklist

**Before accepting AI findings as valid issues:**

```markdown
□ Issue is reproducible with specific inputs/conditions
□ Issue represents actual incorrect behavior, not style preference
□ Issue is not caught by existing static analysis tools
□ Issue has real business/user/security impact
□ Issue is not an intentional design decision (verify with team)
□ Proposed fix actually resolves the issue
□ Proposed fix doesn't introduce new problems
□ Issue hasn't already been documented/known
□ Severity rating is appropriate
□ Root cause analysis is accurate
```

### 7.2 False Positive Handling

**Common AI False Positives:**

```yaml
Pattern Recognition Errors:
  - Flagging intentional patterns as bugs
  - Misunderstanding domain-specific logic
  - Missing context from other files
  - Not recognizing framework conventions
  
Mitigation:
  - Provide more context about architectural decisions
  - Include relevant documentation in analysis
  - Verify findings against actual execution
  - Cross-reference with team knowledge
```

### 7.3 Coverage Metrics

**Track analysis completeness:**

```markdown
Coverage Matrix:
├─ Files Analyzed / Total Files: [%]
├─ Functions Analyzed / Total Functions: [%]
├─ Critical Paths Covered: [%]
├─ Edge Cases Considered: [list]
└─ Known Risks Assessed: [list]
```

---

## Part 8: Integration with Development Workflow

### 8.1 CI/CD Integration Points

```yaml
Pre-Commit:
  - Quick scan of changed files
  - Focus on critical categories only
  - Block on CRITICAL severity findings

Pull Request:
  - Comprehensive analysis of changed code
  - Include related/affected files
  - Report in PR comments

Weekly Audit:
  - Full codebase scan
  - All categories enabled
  - Trend analysis over time

Release Gate:
  - Targeted analysis of release scope
  - Security-focused with CRITICAL/HIGH threshold
  - Must pass before deployment
```

### 8.2 Remediation Workflow

```markdown
1. TRIAGE PHASE:
   - Review all findings
   - Validate authenticity
   - Assign severity and priority
   - Create tracking tickets

2. PLANNING PHASE:
   - Group related issues
   - Estimate effort
   - Schedule fixes
   - Assign ownership

3. IMPLEMENTATION PHASE:
   - Fix issues in priority order
   - Include unit tests for bug scenarios
   - Update documentation if needed
   - Request re-analysis of fixed code

4. VERIFICATION PHASE:
   - Verify fix resolves issue
   - No new issues introduced
   - Existing tests still pass
   - Performance not degraded

5. CLOSURE PHASE:
   - Update issue tracking
   - Document lessons learned
   - Update coding standards if needed
   - Share findings with team
```

---

## Part 9: Customization Extensions

### 9.1 Domain-Specific Extensions

**Create custom analysis modules for your domain:**

```markdown
CUSTOM CATEGORY: [Domain-Specific Analysis]

Detection Focus:
- [Domain-specific anti-pattern 1]
- [Domain-specific anti-pattern 2]
- [Domain-specific rule violation]

Validation Rules:
1. [Rule name]: [Description and check]
2. [Rule name]: [Description and check]

Example Issues:
- [Common mistake in domain]
- [Typical oversight in domain]
```

**Example: E-commerce Domain Extension**
```markdown
CUSTOM CATEGORY: E-commerce Transaction Safety

Detection Focus:
- Shopping cart race conditions (concurrent modifications)
- Inventory overselling (stock not locked during checkout)
- Payment vs. order state inconsistency
- Discount calculation precision errors
- Shipping cost calculation errors
- Tax calculation jurisdiction errors
- Currency conversion rounding issues
- Order state machine violations

Validation Rules:
1. Inventory Check: Every order must verify stock availability atomically
2. Payment Idempotency: Payment operations must be idempotent
3. Order State: Orders must follow valid state transitions only
4. Price Consistency: Display price must match checkout price
```

### 9.2 Framework-Specific Patterns

**Add framework-specific checks:**

```yaml
React Patterns:
  - Hooks called conditionally
  - Missing useEffect dependencies
  - State mutation instead of setState
  - Infinite render loops
  - Missing key props in lists

Django Patterns:
  - N+1 query problems
  - Missing transaction decorators for multi-step operations
  - Incorrect use of select_related/prefetch_related
  - Raw SQL without parameterization

Spring Boot Patterns:
  - Missing @Transactional for data operations
  - Incorrect scope for beans (singleton vs prototype)
  - Lazy initialization issues
  - Missing validation annotations
```

---

## Part 10: Example Workflows

### Example 1: New Feature Analysis

```markdown
SCENARIO: Analyzing newly developed payment processing module

CONFIGURATION:
  focus_areas: "payment_processing module, transaction handling"
  priority_level: CRITICAL
  enabled_categories: [business_logic, security_logic, error_handling, concurrency]
  severity_threshold: MEDIUM

PROMPT ADDITIONS:
  "Pay special attention to:
  - Transaction atomicity and rollback handling
  - Race conditions in concurrent payment attempts
  - Error handling for payment gateway failures
  - Authorization checks before charging
  - Idempotency for retry scenarios
  - Audit logging for all money movements"

EXPECTED OUTPUTS:
  - Transaction boundary analysis
  - Concurrent payment safety verification
  - Error recovery scenario validation
  - Security authorization audit
  - Financial accuracy verification
```

### Example 2: Legacy Code Audit

```markdown
SCENARIO: Auditing 5-year-old authentication module

CONFIGURATION:
  focus_areas: "authentication module, session management, password handling"
  priority_level: HIGH
  enabled_categories: [security_logic, error_handling, architectural_consistency]
  severity_threshold: LOW

PROMPT ADDITIONS:
  "This is legacy code using older patterns. Compare against modern security best practices:
  - Password hashing algorithms (should be bcrypt/argon2, not MD5/SHA1)
  - Session token generation (cryptographically secure random)
  - SQL injection vectors (parameterized queries)
  - Authentication bypass possibilities
  - Timing attack vulnerabilities
  - Session fixation/hijacking vectors"

EXPECTED OUTPUTS:
  - Security vulnerability assessment
  - Outdated pattern identification
  - Modernization recommendations
  - Risk scoring for each finding
```

### Example 3: Performance Critical Path Review

```markdown
SCENARIO: Analyzing high-traffic API endpoint showing latency

CONFIGURATION:
  focus_areas: "ProductSearchAPI, database query layer, caching layer"
  priority_level: HIGH
  enabled_categories: [concurrency_timing, data_state_management, architectural_consistency]
  severity_threshold: MEDIUM

PROMPT ADDITIONS:
  "This endpoint handles 10,000 requests/minute. Focus on:
  - N+1 query problems
  - Missing or incorrect cache usage
  - Unnecessary synchronous operations
  - Lock contention issues
  - Inefficient data structures
  - Missing database indexes implications"

EXPECTED OUTPUTS:
  - Performance bottleneck identification
  - Scalability concerns
  - Optimization opportunities
  - Caching strategy recommendations
```

---

## Part 11: Appendices

### Appendix A: Severity Rating Guidelines

```markdown
🔴 CRITICAL:
- Allows unauthorized access to sensitive data
- Causes data corruption or data loss
- Enables privilege escalation
- Causes financial loss or revenue leakage
- Violates regulatory compliance (GDPR, HIPAA, PCI-DSS)
- System-wide outage or crash
- Critical security vulnerability

🟠 HIGH:
- Causes incorrect business logic execution
- Significant user experience degradation
- Potential data inconsistency
- Performance degradation affecting all users
- Partial system failure
- High-probability edge case causing errors

🟡 MEDIUM:
- Incorrect behavior in uncommon scenarios
- Minor data inconsistency possible
- Performance impact on subset of users
- Moderate technical debt
- Code maintainability concerns
- Missing error handling for non-critical paths

🔵 LOW:
- Code style inconsistencies
- Minor optimization opportunities
- Documentation issues
- Low-probability edge case
- Cosmetic issues
- Minor technical debt
```

### Appendix B: Common Anti-Patterns by Language

```yaml
Python:
  - Mutable default arguments
  - Circular imports
  - Using == for identity checks (should use 'is')
  - Not closing resources (files, connections)
  - Bare except clauses catching all exceptions
  
JavaScript/TypeScript:
  - == instead of === (type coercion)
  - Missing error handling in promises
  - Callback hell / promise hell
  - Not properly binding 'this'
  - Mutating props in React
  
Java:
  - String comparison with ==
  - Not overriding equals() and hashCode() together
  - Catching Exception instead of specific exceptions
  - Not closing resources (before try-with-resources)
  - Synchronizing on non-final objects
  
C#:
  - Not disposing IDisposable objects
  - Async void methods (should be async Task)
  - String concatenation in loops
  - Catching and rethrowing exceptions incorrectly
  - Not using ConfigureAwait(false) in libraries
  
Go:
  - Goroutine closure over loop variable
  - Not checking errors
  - Mutex not protecting all access to shared state
  - Not closing channels
  - Not using context for cancellation
  
Rust:
  - Unwrap() in production code
  - Not handling Result/Option properly
  - Unnecessary cloning
  - Incorrect lifetime annotations
  - Unsafe code without proper invariant documentation
```

### Appendix C: Glossary

```markdown
LOGICAL INCONSISTENCY: Code that is syntactically correct but produces 
incorrect behavior due to flawed reasoning or incorrect assumptions.

INVARIANT: A condition that must always be true at certain points in 
program execution (e.g., account balance must never be negative).

RACE CONDITION: Situation where the outcome depends on the sequence or 
timing of uncontrollable events.

TOCTOU: Time-Of-Check To Time-Of-Use vulnerability where state changes 
between validation and use.

IDEMPOTENCY: Property where an operation produces the same result 
whether executed once or multiple times.

BUSINESS LOGIC: Code that implements domain-specific rules and processes,
as opposed to technical/infrastructure code.

SIDE EFFECT: Observable change in state outside a function's return value
(modifying global state, I/O operations, etc.).

EDGE CASE: Scenario occurring at extreme operating parameters or unusual
combinations of inputs.

STATIC ANALYSIS: Code examination without execution, performed by tools
or manual review.

BOUNDARY CONDITION: Values at the limits of valid input ranges (empty,
zero, maximum, null).
```

---

## Part 12: Prompt Assembly Instructions

### 12.1 Final Prompt Template

**To use this framework, assemble your final AI prompt as follows:**

```markdown
[COPY: Part 2.1 - Role and Context Injection - with filled placeholders]

[COPY: Part 2.2 - Primary Objective - with filled placeholders]

[COPY: Selected categories from Part 3 that are enabled in your config]

[COPY: Part 4.1 - Analysis Methodology]

[COPY: Part 5.1 - Issue Report Template]

ADD:
"CONSTRAINTS AND GUIDELINES:
- Prioritize logical errors that could cause data corruption, security vulnerabilities, or financial impact
- Distinguish between style preferences and actual logical flaws
- Consider the full context of the codebase before flagging patterns that may be intentional
- Provide actionable fixes, not just criticism
- If uncertain about whether something is a bug or intentional design, note it as 'NEEDS_CLARIFICATION'
- Focus on issues that are not caught by static type checkers or linters
- Think step-by-step through complex logic before concluding there is an error
- Use the chain-of-thought reasoning template for each analysis
- Be especially vigilant about issues related to {{CRITICAL_CONSTRAINTS}}"

ADD:
"CODE TO ANALYZE:

[PASTE YOUR CODEBASE OR SPECIFIC FILES HERE]"
```

### 12.2 Execution Checklist

```markdown
BEFORE ANALYSIS:
□ Configuration complete with all placeholders filled
□ Analysis categories selected
□ Severity threshold set
□ Code files prepared and formatted
□ Context documentation included
□ Expected outcomes defined

DURING ANALYSIS:
□ Monitor AI output for relevance
□ Take notes on patterns observed
□ Flag items needing clarification
□ Track time spent on analysis

AFTER ANALYSIS:
□ Review all findings
□ Validate critical issues
□ Create tracking tickets
□ Share with team
□ Document lessons learned
□ Update framework based on experience
```

---

## Conclusion

This Universal Codebase Analysis Framework provides a comprehensive, repeatable system for conducting deep logical analysis on any codebase. By filling in the configuration module and selecting appropriate analysis categories, you can generate tailored prompts that leverage AI capabilities to identify subtle bugs, architectural flaws, and business logic errors.

**Key Principles:**
- **Systematic**: Structured approach covering all major error categories
- **Adaptable**: Configurable for any language, framework, or domain
- **Actionable**: Provides specific fixes, not just problem identification
- **Validated**: Includes quality assurance and false positive handling
- **Integrated**: Fits into existing development workflows

**Remember:** AI-assisted analysis is a powerful tool but requires human validation and judgment. Use this framework as a force multiplier for code review, not a replacement for human expertise.

---

*Framework Version: 1.0*  
*Last Updated: 2024*  
*Maintained by: [Your Organization]*
