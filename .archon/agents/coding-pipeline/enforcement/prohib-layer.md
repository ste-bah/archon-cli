---
name: PROHIB Enforcement Layer
type: enforcement-layer
color: "#D32F2F"
description: Prohibition detection and enforcement system - prevents dangerous operations, security violations, and architectural breaches across all pipeline phases
category: enforcement
version: 1.0.0
priority: 0
phase: all
capabilities:
  - prohibition-detection
  - violation-enforcement
  - security-boundary-protection
  - resource-limit-enforcement
  - architectural-constraint-enforcement
  - quality-floor-enforcement
hooks:
  pre:
    - action: validate
      rules: all-prohibitions
  post:
    - action: store
      key: coding/prohib/violations
    - action: notify
      channel: security-alerts
      condition: violation-detected
---

# PROHIB Enforcement Layer

## Overview

The PROHIB (Prohibition) Enforcement Layer implements 8 critical prohibition mechanisms that prevent dangerous, insecure, or architecturally unsound operations. Unlike quality gates that use thresholds, PROHIB rules are binary - any violation immediately halts execution and triggers escalation.

---

## PROHIB RULE ENFORCEMENT MATRIX

This matrix documents which agents enforce which PROHIB rules, creating bidirectional references between rules and their enforcers.

| PROHIB Rule | Description | Enforcing Agents | Enforcement Phase |
|-------------|-------------|------------------|-------------------|
| PROHIB-1 | Security Violation Prevention | `task-analyzer`, `code-generator`, `sign-off-approver` | Analysis, Implementation, Sign-off |
| PROHIB-2 | Resource Exhaustion Prevention | `task-analyzer`, `code-generator`, `test-runner` | Analysis, Implementation, Testing |
| PROHIB-3 | Architectural Constraint Enforcement | `task-analyzer`, `code-generator`, `sign-off-approver` | Analysis, Implementation, Sign-off |
| PROHIB-4 | Quality Floor Enforcement | `test-runner`, `sign-off-approver` | Testing, Sign-off |
| PROHIB-5 | Data Integrity Protection | `code-generator`, `test-runner` | Implementation, Testing |
| PROHIB-6 | External Boundary Protection | `task-analyzer`, `code-generator`, `sign-off-approver` | Analysis, Implementation, Sign-off |
| PROHIB-7 | Behavioral Constraints | `test-runner`, `code-generator` | Testing, Implementation |
| PROHIB-8 | Process Compliance Enforcement | `sign-off-approver` | Sign-off |

### Agent-to-PROHIB Cross-Reference

| Agent | PROHIB Rules Enforced | Primary Responsibility |
|-------|----------------------|------------------------|
| `task-analyzer` | PROHIB-1, PROHIB-2, PROHIB-3, PROHIB-6 | Pre-implementation security/resource/architecture validation |
| `code-generator` | PROHIB-1, PROHIB-2, PROHIB-3, PROHIB-5, PROHIB-6, PROHIB-7 | Implementation-time constraint enforcement |
| `test-runner` | PROHIB-2, PROHIB-4, PROHIB-5, PROHIB-7 | Runtime validation and quality floor checks |
| `sign-off-approver` | PROHIB-1, PROHIB-3, PROHIB-4, PROHIB-6, PROHIB-8 | Final gate enforcement and compliance verification |

---

## ENFORCEMENT VALIDATION

Each enforcing agent MUST adhere to the following validation protocol:

### Pre-Action Validation
```typescript
interface ProhibPreValidation {
  checkApplicableRules(): ProhibRule[];
  validateConstraints(context: ExecutionContext): ValidationResult;
  blockOnViolation(violation: ProhibViolation): never;
}
```

### Required Agent Behaviors

1. **Check applicable PROHIB rules before action execution**
   - Retrieve PROHIB rule definitions from `coding/prohib/rules`
   - Determine which rules apply to current operation
   - Execute rule-specific validation logic

2. **Log PROHIB validation results to memory**
   ```bash
   mcp__memorygraph__store_memory with title "coding/prohib/validation/<agent>/<timestamp>", content '<result>', tags ["coding-pipeline", "prohib"]
   ```

3. **Escalate violations via EMERG triggers**
   - Security violations (PROHIB-1) → `EMERG-SECURITY`
   - Resource exhaustion (PROHIB-2) → `EMERG-RESOURCE`
   - Quality floor breach (PROHIB-4) → `EMERG-QUALITY`
   - Process compliance (PROHIB-8) → `EMERG-COMPLIANCE`

4. **Block action if PROHIB violation detected**
   - Immediately halt execution
   - Store violation in `coding/prohib/violations`
   - Return `{ proceed: false, violation: ProhibViolation }`

### EMERG Trigger Integration

| PROHIB Violation | EMERG Trigger | Response |
|-----------------|---------------|----------|
| PROHIB-1 Critical | `EMERG-SECURITY` | Immediate halt, security review required |
| PROHIB-2 Critical | `EMERG-RESOURCE` | Halt, resource optimization required |
| PROHIB-3 Critical | `EMERG-ARCHITECTURE` | Halt, architecture review required |
| PROHIB-4 Any | `EMERG-QUALITY` | Halt, quality remediation required |
| PROHIB-5 Any | `EMERG-DATA-INTEGRITY` | Halt, data protection review |
| PROHIB-6 Critical | `EMERG-BOUNDARY` | Halt, boundary violation review |
| PROHIB-7 High | `EMERG-BEHAVIORAL` | Halt, behavior correction required |
| PROHIB-8 Any | `EMERG-COMPLIANCE` | Halt, process compliance review |

---

## Cross-Reference to Agent Files

The following agent files contain PROHIB enforcement logic and reference this document:

- **Analysis Phase**: `agents/task-analyzer.md` - Pre-implementation PROHIB validation
- **Implementation Phase**: `agents/code-generator.md` - Code-time PROHIB enforcement
- **Testing Phase**: `agents/test-runner.md` - Runtime PROHIB validation
- **Sign-off Phase**: `agents/sign-off-approver.md` - Final PROHIB gate

### Memory Keys for PROHIB Coordination

| Memory Key | Purpose | Updated By |
|------------|---------|------------|
| `coding/prohib/rules` | Rule definitions | System |
| `coding/prohib/violations` | Violation records | All enforcing agents |
| `coding/prohib/validation/<agent>` | Validation results | Individual agents |
| `coding/prohib/escalations` | EMERG escalation records | ViolationHandler |

---

## Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                    PROHIB ENFORCEMENT LAYER                      │
├─────────────────────────────────────────────────────────────────┤
│  ┌─────────────┐ ┌─────────────┐ ┌─────────────┐ ┌────────────┐│
│  │   PROHIB-1  │ │   PROHIB-2  │ │   PROHIB-3  │ │  PROHIB-4  ││
│  │  Security   │ │  Resource   │ │Architecture │ │  Quality   ││
│  │ Violations  │ │ Exhaustion  │ │  Breaches   │ │   Floor    ││
│  └──────┬──────┘ └──────┬──────┘ └──────┬──────┘ └──────┬─────┘│
│         │               │               │               │       │
│  ┌──────┴──────┐ ┌──────┴──────┐ ┌──────┴──────┐ ┌──────┴─────┐│
│  │   PROHIB-5  │ │   PROHIB-6  │ │   PROHIB-7  │ │  PROHIB-8  ││
│  │    Data     │ │  External   │ │  Behavioral │ │  Process   ││
│  │  Integrity  │ │  Boundary   │ │ Constraints │ │ Compliance ││
│  └─────────────┘ └─────────────┘ └─────────────┘ └────────────┘│
├─────────────────────────────────────────────────────────────────┤
│                    VIOLATION HANDLER                             │
│  ┌───────────┐ ┌───────────┐ ┌───────────┐ ┌────────────────┐  │
│  │  Detect   │→│   Halt    │→│  Notify   │→│  Escalate/Log  │  │
│  └───────────┘ └───────────┘ └───────────┘ └────────────────┘  │
└─────────────────────────────────────────────────────────────────┘
```

## PROHIB-1: Security Violation Prevention

Prevents security vulnerabilities and unsafe code patterns.

```typescript
export interface SecurityViolation {
  type: SecurityViolationType;
  severity: 'critical' | 'high';
  location: CodeLocation;
  description: string;
  cweId?: string;
  remediation: string;
}

export enum SecurityViolationType {
  HARDCODED_SECRET = 'hardcoded_secret',
  SQL_INJECTION = 'sql_injection',
  COMMAND_INJECTION = 'command_injection',
  XSS_VULNERABILITY = 'xss_vulnerability',
  PATH_TRAVERSAL = 'path_traversal',
  INSECURE_DESERIALIZATION = 'insecure_deserialization',
  BROKEN_AUTH = 'broken_auth',
  SENSITIVE_DATA_EXPOSURE = 'sensitive_data_exposure',
  INSECURE_CRYPTO = 'insecure_crypto',
  EVAL_USAGE = 'eval_usage',
}

export class SecurityViolationDetector {
  private readonly patterns: Map<SecurityViolationType, RegExp[]> = new Map([
    [SecurityViolationType.HARDCODED_SECRET, [
      /(?:password|secret|api_?key|token|auth)[\s]*[=:]["'][^"']{8,}["']/gi,
      /-----BEGIN (?:RSA|DSA|EC|OPENSSH) PRIVATE KEY-----/,
      /AIza[0-9A-Za-z-_]{35}/, // Google API Key
      /sk-[a-zA-Z0-9]{48}/, // OpenAI API Key
      /ghp_[a-zA-Z0-9]{36}/, // GitHub Personal Access Token
    ]],
    [SecurityViolationType.SQL_INJECTION, [
      /\$\{.*\}.*(?:SELECT|INSERT|UPDATE|DELETE|DROP|UNION)/i,
      /['"]?\s*\+\s*\w+\s*\+\s*['"]?.*(?:SELECT|INSERT|UPDATE|DELETE)/i,
      /(?:query|execute)\s*\(\s*['"`].*\$\{/i,
    ]],
    [SecurityViolationType.COMMAND_INJECTION, [
      /(?:exec|spawn|execSync|spawnSync)\s*\([^)]*\$\{/,
      /child_process.*\+.*(?:input|param|arg|user)/i,
      /(?:system|popen|shell_exec)\s*\([^)]*\$(?:_GET|_POST|_REQUEST)/,
    ]],
    [SecurityViolationType.EVAL_USAGE, [
      /\beval\s*\(/,
      /new\s+Function\s*\(/,
      /setTimeout\s*\(\s*['"`][^'"`)]+['"`]/,
    ]],
    [SecurityViolationType.PATH_TRAVERSAL, [
      /(?:readFile|writeFile|unlink|rmdir)\s*\([^)]*(?:req\.|input|param)/i,
      /\.\.\/.*(?:readFile|writeFile|access)/,
    ]],
  ]);

  private readonly cweMapping: Map<SecurityViolationType, string> = new Map([
    [SecurityViolationType.HARDCODED_SECRET, 'CWE-798'],
    [SecurityViolationType.SQL_INJECTION, 'CWE-89'],
    [SecurityViolationType.COMMAND_INJECTION, 'CWE-78'],
    [SecurityViolationType.XSS_VULNERABILITY, 'CWE-79'],
    [SecurityViolationType.PATH_TRAVERSAL, 'CWE-22'],
    [SecurityViolationType.INSECURE_DESERIALIZATION, 'CWE-502'],
    [SecurityViolationType.EVAL_USAGE, 'CWE-95'],
  ]);

  async detectViolations(code: string, filePath: string): Promise<SecurityViolation[]> {
    const violations: SecurityViolation[] = [];

    for (const [type, patterns] of this.patterns) {
      for (const pattern of patterns) {
        const matches = code.matchAll(new RegExp(pattern, 'gm'));

        for (const match of matches) {
          const lineNumber = this.getLineNumber(code, match.index!);

          violations.push({
            type,
            severity: this.getSeverity(type),
            location: { file: filePath, line: lineNumber, column: match.index! },
            description: this.getDescription(type, match[0]),
            cweId: this.cweMapping.get(type),
            remediation: this.getRemediation(type),
          });
        }
      }
    }

    return violations;
  }

  private getSeverity(type: SecurityViolationType): 'critical' | 'high' {
    const criticalTypes = [
      SecurityViolationType.HARDCODED_SECRET,
      SecurityViolationType.SQL_INJECTION,
      SecurityViolationType.COMMAND_INJECTION,
    ];
    return criticalTypes.includes(type) ? 'critical' : 'high';
  }

  private getDescription(type: SecurityViolationType, match: string): string {
    switch (type) {
      case SecurityViolationType.HARDCODED_SECRET:
        return `Hardcoded secret detected: ${match.substring(0, 20)}...`;
      case SecurityViolationType.SQL_INJECTION:
        return 'Potential SQL injection vulnerability - user input concatenated in query';
      case SecurityViolationType.COMMAND_INJECTION:
        return 'Potential command injection - user input passed to shell execution';
      case SecurityViolationType.EVAL_USAGE:
        return 'Dangerous eval() usage detected';
      default:
        return `Security violation: ${type}`;
    }
  }

  private getRemediation(type: SecurityViolationType): string {
    switch (type) {
      case SecurityViolationType.HARDCODED_SECRET:
        return 'Use environment variables or secret management service';
      case SecurityViolationType.SQL_INJECTION:
        return 'Use parameterized queries or prepared statements';
      case SecurityViolationType.COMMAND_INJECTION:
        return 'Validate and sanitize input, use allowlists, avoid shell execution';
      case SecurityViolationType.EVAL_USAGE:
        return 'Replace eval with safer alternatives like JSON.parse or specific parsers';
      default:
        return 'Review and fix the security issue';
    }
  }
}
```

## PROHIB-2: Resource Exhaustion Prevention

Prevents resource exhaustion and denial-of-service conditions.

```typescript
export interface ResourceLimit {
  type: ResourceType;
  limit: number;
  unit: string;
  current: number;
  exceeded: boolean;
}

export enum ResourceType {
  MEMORY = 'memory',
  CPU = 'cpu',
  FILE_SIZE = 'file_size',
  FILE_COUNT = 'file_count',
  EXECUTION_TIME = 'execution_time',
  RECURSION_DEPTH = 'recursion_depth',
  LOOP_ITERATIONS = 'loop_iterations',
  CONCURRENT_OPERATIONS = 'concurrent_operations',
}

export class ResourceExhaustionPreventer {
  private readonly limits: Map<ResourceType, ResourceLimit> = new Map([
    [ResourceType.MEMORY, { type: ResourceType.MEMORY, limit: 512, unit: 'MB', current: 0, exceeded: false }],
    [ResourceType.CPU, { type: ResourceType.CPU, limit: 80, unit: '%', current: 0, exceeded: false }],
    [ResourceType.FILE_SIZE, { type: ResourceType.FILE_SIZE, limit: 500, unit: 'lines', current: 0, exceeded: false }],
    [ResourceType.FILE_COUNT, { type: ResourceType.FILE_COUNT, limit: 100, unit: 'files', current: 0, exceeded: false }],
    [ResourceType.EXECUTION_TIME, { type: ResourceType.EXECUTION_TIME, limit: 300, unit: 'seconds', current: 0, exceeded: false }],
    [ResourceType.RECURSION_DEPTH, { type: ResourceType.RECURSION_DEPTH, limit: 100, unit: 'levels', current: 0, exceeded: false }],
    [ResourceType.LOOP_ITERATIONS, { type: ResourceType.LOOP_ITERATIONS, limit: 10000, unit: 'iterations', current: 0, exceeded: false }],
    [ResourceType.CONCURRENT_OPERATIONS, { type: ResourceType.CONCURRENT_OPERATIONS, limit: 50, unit: 'operations', current: 0, exceeded: false }],
  ]);

  private readonly codePatterns = {
    infiniteLoop: [
      /while\s*\(\s*true\s*\)/,
      /for\s*\(\s*;\s*;\s*\)/,
      /do\s*\{[^}]*\}\s*while\s*\(\s*true\s*\)/,
    ],
    unboundedRecursion: [
      /function\s+(\w+)\s*\([^)]*\)\s*\{[^}]*\1\s*\([^)]*\)/,
    ],
    largeAllocation: [
      /new\s+Array\s*\(\s*\d{7,}\s*\)/,
      /Buffer\.alloc\s*\(\s*\d{9,}\s*\)/,
    ],
    resourceLeak: [
      /(?:createReadStream|createWriteStream|open|connect)\s*\([^)]*\)(?![^;]*\.on\s*\(\s*['"](?:close|end|error)['"]\s*,)/,
    ],
  };

  async checkCodePatterns(code: string): Promise<ResourceViolation[]> {
    const violations: ResourceViolation[] = [];

    // Check for infinite loops
    for (const pattern of this.codePatterns.infiniteLoop) {
      if (pattern.test(code)) {
        violations.push({
          type: ResourceType.LOOP_ITERATIONS,
          description: 'Potential infinite loop detected',
          severity: 'critical',
          pattern: pattern.toString(),
        });
      }
    }

    // Check for unbounded recursion
    for (const pattern of this.codePatterns.unboundedRecursion) {
      if (pattern.test(code)) {
        violations.push({
          type: ResourceType.RECURSION_DEPTH,
          description: 'Potential unbounded recursion without base case',
          severity: 'high',
          pattern: pattern.toString(),
        });
      }
    }

    // Check for large allocations
    for (const pattern of this.codePatterns.largeAllocation) {
      if (pattern.test(code)) {
        violations.push({
          type: ResourceType.MEMORY,
          description: 'Extremely large memory allocation detected',
          severity: 'critical',
          pattern: pattern.toString(),
        });
      }
    }

    // Check for resource leaks
    for (const pattern of this.codePatterns.resourceLeak) {
      const matches = code.match(pattern);
      if (matches) {
        violations.push({
          type: ResourceType.MEMORY,
          description: 'Potential resource leak - stream/handle not properly closed',
          severity: 'high',
          pattern: pattern.toString(),
        });
      }
    }

    return violations;
  }

  async checkRuntimeLimits(): Promise<ResourceLimit[]> {
    const exceeded: ResourceLimit[] = [];

    // Check memory usage
    const memoryUsage = process.memoryUsage();
    const memoryMB = memoryUsage.heapUsed / 1024 / 1024;
    const memoryLimit = this.limits.get(ResourceType.MEMORY)!;
    memoryLimit.current = memoryMB;
    if (memoryMB > memoryLimit.limit) {
      memoryLimit.exceeded = true;
      exceeded.push(memoryLimit);
    }

    return exceeded;
  }

  enforceFileLimit(lineCount: number, filePath: string): boolean {
    const limit = this.limits.get(ResourceType.FILE_SIZE)!;
    if (lineCount > limit.limit) {
      throw new ProhibViolationError(
        'PROHIB-2',
        `File ${filePath} exceeds ${limit.limit} line limit (has ${lineCount} lines)`
      );
    }
    return true;
  }
}
```

## PROHIB-3: Architectural Constraint Enforcement

Enforces architectural boundaries and design constraints.

```typescript
export interface ArchitecturalRule {
  id: string;
  name: string;
  description: string;
  constraint: ArchitecturalConstraint;
  severity: 'critical' | 'high';
}

export type ArchitecturalConstraint =
  | { type: 'layer_violation'; sourceLayer: string; targetLayer: string }
  | { type: 'circular_dependency'; modules: string[] }
  | { type: 'forbidden_import'; from: string; forbidden: string[] }
  | { type: 'coupling_limit'; maxAfferentCoupling: number; maxEfferentCoupling: number }
  | { type: 'component_size'; maxFiles: number; maxLines: number };

export class ArchitecturalConstraintEnforcer {
  private readonly layerOrder = ['presentation', 'application', 'domain', 'infrastructure'];

  private readonly forbiddenImports: Map<string, string[]> = new Map([
    ['domain', ['infrastructure', 'presentation', 'express', 'fastify', 'pg', 'mongodb']],
    ['application', ['presentation', 'express', 'fastify']],
  ]);

  private readonly couplingLimits = {
    maxAfferentCoupling: 20,  // Max incoming dependencies
    maxEfferentCoupling: 10,  // Max outgoing dependencies
  };

  async enforceLayerBoundaries(
    sourceFile: string,
    imports: string[]
  ): Promise<ArchitecturalViolation[]> {
    const violations: ArchitecturalViolation[] = [];
    const sourceLayer = this.getLayer(sourceFile);

    for (const importPath of imports) {
      const targetLayer = this.getLayer(importPath);

      if (this.isLayerViolation(sourceLayer, targetLayer)) {
        violations.push({
          rule: 'layer_violation',
          severity: 'critical',
          source: sourceFile,
          target: importPath,
          message: `Layer violation: ${sourceLayer} cannot import from ${targetLayer}`,
          remediation: 'Move the dependency to a lower layer or use dependency inversion',
        });
      }
    }

    // Check forbidden imports
    const forbidden = this.forbiddenImports.get(sourceLayer);
    if (forbidden) {
      for (const importPath of imports) {
        for (const forbiddenPattern of forbidden) {
          if (importPath.includes(forbiddenPattern)) {
            violations.push({
              rule: 'forbidden_import',
              severity: 'high',
              source: sourceFile,
              target: importPath,
              message: `Forbidden import in ${sourceLayer}: cannot import ${forbiddenPattern}`,
              remediation: 'Use abstraction/interface instead of concrete implementation',
            });
          }
        }
      }
    }

    return violations;
  }

  private getLayer(filePath: string): string {
    for (const layer of this.layerOrder) {
      if (filePath.includes(`/${layer}/`) || filePath.includes(`/${layer}s/`)) {
        return layer;
      }
    }
    return 'unknown';
  }

  private isLayerViolation(sourceLayer: string, targetLayer: string): boolean {
    const sourceIndex = this.layerOrder.indexOf(sourceLayer);
    const targetIndex = this.layerOrder.indexOf(targetLayer);

    if (sourceIndex === -1 || targetIndex === -1) return false;

    // Higher layers cannot import from lower layers (except infrastructure)
    // Domain cannot import from anything above it
    if (sourceLayer === 'domain' && targetIndex < sourceIndex) {
      return true;
    }

    // No layer can import from presentation except presentation
    if (targetLayer === 'presentation' && sourceLayer !== 'presentation') {
      return true;
    }

    return false;
  }

  async detectCircularDependencies(
    dependencyGraph: Map<string, string[]>
  ): Promise<string[][]> {
    const cycles: string[][] = [];
    const visited = new Set<string>();
    const recursionStack = new Set<string>();
    const path: string[] = [];

    const dfs = (node: string): boolean => {
      visited.add(node);
      recursionStack.add(node);
      path.push(node);

      const dependencies = dependencyGraph.get(node) || [];

      for (const dep of dependencies) {
        if (!visited.has(dep)) {
          if (dfs(dep)) return true;
        } else if (recursionStack.has(dep)) {
          // Found cycle
          const cycleStart = path.indexOf(dep);
          const cycle = path.slice(cycleStart);
          cycle.push(dep);
          cycles.push(cycle);
        }
      }

      path.pop();
      recursionStack.delete(node);
      return false;
    };

    for (const node of dependencyGraph.keys()) {
      if (!visited.has(node)) {
        dfs(node);
      }
    }

    return cycles;
  }

  async enforceCouplingLimits(
    module: string,
    afferentCoupling: number,
    efferentCoupling: number
  ): Promise<ArchitecturalViolation[]> {
    const violations: ArchitecturalViolation[] = [];

    if (afferentCoupling > this.couplingLimits.maxAfferentCoupling) {
      violations.push({
        rule: 'coupling_limit',
        severity: 'high',
        source: module,
        target: '',
        message: `High afferent coupling (${afferentCoupling} > ${this.couplingLimits.maxAfferentCoupling}): too many modules depend on this`,
        remediation: 'Consider breaking this module into smaller, more focused modules',
      });
    }

    if (efferentCoupling > this.couplingLimits.maxEfferentCoupling) {
      violations.push({
        rule: 'coupling_limit',
        severity: 'high',
        source: module,
        target: '',
        message: `High efferent coupling (${efferentCoupling} > ${this.couplingLimits.maxEfferentCoupling}): this module depends on too many others`,
        remediation: 'Apply facade pattern or consolidate related dependencies',
      });
    }

    return violations;
  }
}
```

## PROHIB-4: Quality Floor Enforcement

Enforces minimum quality thresholds that cannot be violated.

```typescript
export interface QualityFloor {
  metric: string;
  minimum: number;
  current: number;
  violated: boolean;
}

export class QualityFloorEnforcer {
  private readonly floors: Map<string, number> = new Map([
    ['test_coverage', 60],           // Minimum 60% test coverage
    ['maintainability_index', 40],   // Minimum maintainability
    ['documentation_coverage', 50],  // Minimum doc coverage for public APIs
    ['type_coverage', 70],           // Minimum TypeScript strict coverage
    ['lint_score', 80],              // Minimum lint passing rate
    ['security_score', 90],          // Minimum security audit score
    ['accessibility_score', 70],     // Minimum a11y compliance
    ['performance_budget', 0],       // No performance regressions allowed
  ]);

  async checkQualityFloors(metrics: Record<string, number>): Promise<QualityFloor[]> {
    const violations: QualityFloor[] = [];

    for (const [metric, minimum] of this.floors) {
      const current = metrics[metric];

      if (current !== undefined && current < minimum) {
        violations.push({
          metric,
          minimum,
          current,
          violated: true,
        });
      }
    }

    return violations;
  }

  enforceFloor(metric: string, value: number): void {
    const minimum = this.floors.get(metric);

    if (minimum !== undefined && value < minimum) {
      throw new ProhibViolationError(
        'PROHIB-4',
        `Quality floor violated: ${metric} is ${value}, minimum is ${minimum}`
      );
    }
  }

  // Specific enforcers for common metrics
  async enforceTestCoverage(coverageReport: CoverageReport): Promise<void> {
    const minimum = this.floors.get('test_coverage')!;

    if (coverageReport.overall < minimum) {
      throw new ProhibViolationError(
        'PROHIB-4',
        `Test coverage ${coverageReport.overall}% is below minimum ${minimum}%`
      );
    }

    // Also check critical paths must have 100% coverage
    for (const [path, coverage] of Object.entries(coverageReport.criticalPaths)) {
      if (coverage < 100) {
        throw new ProhibViolationError(
          'PROHIB-4',
          `Critical path ${path} has ${coverage}% coverage, must be 100%`
        );
      }
    }
  }

  async enforceNoPerformanceRegression(
    baseline: PerformanceMetrics,
    current: PerformanceMetrics
  ): Promise<void> {
    const regressionThreshold = 0.05; // 5% regression tolerance

    for (const [metric, baselineValue] of Object.entries(baseline)) {
      const currentValue = current[metric as keyof PerformanceMetrics];
      const regression = (currentValue - baselineValue) / baselineValue;

      if (regression > regressionThreshold) {
        throw new ProhibViolationError(
          'PROHIB-4',
          `Performance regression detected: ${metric} regressed by ${(regression * 100).toFixed(1)}%`
        );
      }
    }
  }
}
```

## PROHIB-5: Data Integrity Protection

Protects data integrity and prevents corruption.

```typescript
export class DataIntegrityProtector {
  private readonly prohibitedOperations = [
    { pattern: /DELETE\s+FROM\s+\w+\s*(?:;|$)/i, description: 'DELETE without WHERE clause' },
    { pattern: /UPDATE\s+\w+\s+SET\s+.*(?:;|$)(?!.*WHERE)/i, description: 'UPDATE without WHERE clause' },
    { pattern: /DROP\s+(?:TABLE|DATABASE|SCHEMA)/i, description: 'DROP operation' },
    { pattern: /TRUNCATE\s+TABLE/i, description: 'TRUNCATE operation' },
    { pattern: /ALTER\s+TABLE.*DROP\s+COLUMN/i, description: 'DROP COLUMN operation' },
  ];

  private readonly requiredValidations = [
    { context: 'user_input', validations: ['sanitize', 'validate_type', 'validate_length'] },
    { context: 'file_upload', validations: ['validate_type', 'validate_size', 'scan_malware'] },
    { context: 'api_response', validations: ['validate_schema', 'sanitize_output'] },
  ];

  async checkDangerousOperations(code: string): Promise<DataIntegrityViolation[]> {
    const violations: DataIntegrityViolation[] = [];

    for (const op of this.prohibitedOperations) {
      if (op.pattern.test(code)) {
        violations.push({
          type: 'dangerous_operation',
          description: `Prohibited operation detected: ${op.description}`,
          severity: 'critical',
          remediation: 'Add appropriate safeguards or use soft-delete pattern',
        });
      }
    }

    return violations;
  }

  async checkInputValidation(
    code: string,
    context: string
  ): Promise<DataIntegrityViolation[]> {
    const violations: DataIntegrityViolation[] = [];
    const required = this.requiredValidations.find(r => r.context === context);

    if (!required) return violations;

    for (const validation of required.validations) {
      const hasValidation = this.checkForValidation(code, validation);

      if (!hasValidation) {
        violations.push({
          type: 'missing_validation',
          description: `Missing required validation: ${validation} for ${context}`,
          severity: 'high',
          remediation: `Add ${validation} validation before processing ${context}`,
        });
      }
    }

    return violations;
  }

  private checkForValidation(code: string, validationType: string): boolean {
    const patterns: Record<string, RegExp> = {
      sanitize: /(?:sanitize|escape|encode|htmlspecialchars|DOMPurify)/i,
      validate_type: /(?:typeof|instanceof|is[A-Z]\w+|validator\.|zod|yup|joi)/i,
      validate_length: /(?:\.length\s*[<>=]|maxLength|minLength|max:|min:)/i,
      validate_size: /(?:\.size\s*[<>=]|maxSize|fileSize)/i,
      validate_schema: /(?:\.parse\(|\.validate\(|ajv|jsonschema)/i,
      scan_malware: /(?:clamav|virusTotal|malware|scan)/i,
      sanitize_output: /(?:escape|encode|sanitize).*(?:output|response|render)/i,
    };

    return patterns[validationType]?.test(code) ?? false;
  }

  async enforceImmutability(
    originalState: unknown,
    newState: unknown,
    immutablePaths: string[]
  ): Promise<void> {
    for (const path of immutablePaths) {
      const originalValue = this.getValueByPath(originalState, path);
      const newValue = this.getValueByPath(newState, path);

      if (!this.deepEqual(originalValue, newValue)) {
        throw new ProhibViolationError(
          'PROHIB-5',
          `Immutable data modified: ${path}`
        );
      }
    }
  }
}
```

## PROHIB-6: External Boundary Protection

Protects against unauthorized external access and API abuse.

```typescript
export class ExternalBoundaryProtector {
  private readonly allowedDomains: Set<string> = new Set([
    'api.github.com',
    'registry.npmjs.org',
    'cdn.jsdelivr.net',
  ]);

  private readonly blockedPatterns: RegExp[] = [
    /(?:localhost|127\.0\.0\.1|0\.0\.0\.0)/,  // Local addresses
    /(?:192\.168\.|10\.|172\.(?:1[6-9]|2\d|3[01])\.)/,  // Private networks
    /(?:metadata\.google|169\.254\.)/,  // Cloud metadata endpoints
  ];

  private readonly rateLimits: Map<string, RateLimit> = new Map([
    ['api_calls', { limit: 1000, window: 3600, current: 0 }],
    ['file_operations', { limit: 500, window: 60, current: 0 }],
    ['network_requests', { limit: 100, window: 60, current: 0 }],
  ]);

  async validateExternalUrl(url: string): Promise<ExternalViolation[]> {
    const violations: ExternalViolation[] = [];

    // Check against blocked patterns
    for (const pattern of this.blockedPatterns) {
      if (pattern.test(url)) {
        violations.push({
          type: 'blocked_destination',
          url,
          description: 'Access to internal/private endpoints is prohibited',
          severity: 'critical',
        });
      }
    }

    // Check against allowed domains for external APIs
    try {
      const urlObj = new URL(url);
      if (!this.allowedDomains.has(urlObj.hostname) && !url.startsWith('file://')) {
        violations.push({
          type: 'unapproved_domain',
          url,
          description: `Domain ${urlObj.hostname} is not in the approved list`,
          severity: 'high',
        });
      }
    } catch {
      violations.push({
        type: 'invalid_url',
        url,
        description: 'Invalid URL format',
        severity: 'high',
      });
    }

    return violations;
  }

  async checkRateLimit(operation: string): Promise<boolean> {
    const limit = this.rateLimits.get(operation);
    if (!limit) return true;

    limit.current++;

    if (limit.current > limit.limit) {
      throw new ProhibViolationError(
        'PROHIB-6',
        `Rate limit exceeded for ${operation}: ${limit.current}/${limit.limit} per ${limit.window}s`
      );
    }

    return true;
  }

  async validateApiUsage(
    endpoint: string,
    method: string,
    headers: Record<string, string>
  ): Promise<void> {
    // Check for required authentication
    if (!headers['Authorization'] && !headers['X-API-Key']) {
      throw new ProhibViolationError(
        'PROHIB-6',
        'External API calls require authentication headers'
      );
    }

    // Check for dangerous methods on production endpoints
    if (['DELETE', 'PUT', 'PATCH'].includes(method.toUpperCase())) {
      if (endpoint.includes('prod') || endpoint.includes('production')) {
        throw new ProhibViolationError(
          'PROHIB-6',
          'Destructive operations on production endpoints are prohibited'
        );
      }
    }
  }
}
```

## PROHIB-7: Behavioral Constraints

Enforces behavioral constraints and invariants.

```typescript
export class BehavioralConstraintEnforcer {
  private readonly invariants: Map<string, Invariant> = new Map([
    ['no_side_effects_in_getters', {
      check: (code: string) => !/get\s+\w+\s*\(\)\s*\{[^}]*(?:this\.\w+\s*=|console\.|fetch\(|await)/.test(code),
      message: 'Getters must not have side effects',
    }],
    ['async_error_handling', {
      check: (code: string) => !/async\s+\w+[^{]*\{(?:(?!try\s*\{|\.catch\()[\s\S])*await\s+/.test(code) ||
                              /async\s+\w+[^{]*\{[\s\S]*try\s*\{[\s\S]*await/.test(code),
      message: 'Async functions must handle errors with try-catch or .catch()',
    }],
    ['no_floating_promises', {
      check: (code: string) => !/(?<!await\s+|return\s+|=\s*)\w+\([^)]*\)\s*;(?!\s*\/\/\s*fire-and-forget)/.test(code) ||
                              true, // Complex check, simplified
      message: 'Promises must be awaited, returned, or explicitly marked as fire-and-forget',
    }],
    ['constructor_no_async', {
      check: (code: string) => !/constructor\s*\([^)]*\)\s*\{[^}]*await\s+/.test(code),
      message: 'Constructors cannot contain await - use factory pattern instead',
    }],
  ]);

  private readonly stateTransitions: Map<string, ValidTransitions> = new Map([
    ['order', {
      pending: ['confirmed', 'cancelled'],
      confirmed: ['processing', 'cancelled'],
      processing: ['shipped', 'failed'],
      shipped: ['delivered', 'returned'],
      delivered: ['returned'],
      cancelled: [],
      failed: ['pending'],
      returned: ['refunded'],
      refunded: [],
    }],
  ]);

  async checkInvariants(code: string): Promise<BehavioralViolation[]> {
    const violations: BehavioralViolation[] = [];

    for (const [name, invariant] of this.invariants) {
      if (!invariant.check(code)) {
        violations.push({
          type: 'invariant_violation',
          invariant: name,
          message: invariant.message,
          severity: 'high',
        });
      }
    }

    return violations;
  }

  validateStateTransition(
    entity: string,
    fromState: string,
    toState: string
  ): void {
    const transitions = this.stateTransitions.get(entity);
    if (!transitions) return;

    const validTargets = transitions[fromState];
    if (!validTargets) {
      throw new ProhibViolationError(
        'PROHIB-7',
        `Unknown state '${fromState}' for entity '${entity}'`
      );
    }

    if (!validTargets.includes(toState)) {
      throw new ProhibViolationError(
        'PROHIB-7',
        `Invalid state transition for ${entity}: ${fromState} -> ${toState}. Valid targets: ${validTargets.join(', ')}`
      );
    }
  }

  async enforceContractPreConditions(
    methodName: string,
    args: unknown[],
    preConditions: PreCondition[]
  ): Promise<void> {
    for (const condition of preConditions) {
      if (!condition.check(...args)) {
        throw new ProhibViolationError(
          'PROHIB-7',
          `Pre-condition failed for ${methodName}: ${condition.message}`
        );
      }
    }
  }

  async enforceContractPostConditions(
    methodName: string,
    result: unknown,
    postConditions: PostCondition[]
  ): Promise<void> {
    for (const condition of postConditions) {
      if (!condition.check(result)) {
        throw new ProhibViolationError(
          'PROHIB-7',
          `Post-condition failed for ${methodName}: ${condition.message}`
        );
      }
    }
  }
}
```

## PROHIB-8: Process Compliance Enforcement

Enforces process and workflow compliance.

```typescript
export class ProcessComplianceEnforcer {
  private readonly requiredSteps: Map<string, string[]> = new Map([
    ['code_change', ['lint', 'type_check', 'unit_test', 'code_review']],
    ['deployment', ['integration_test', 'security_scan', 'approval', 'backup']],
    ['database_migration', ['backup', 'review', 'test_rollback', 'approval']],
    ['api_change', ['version_check', 'backward_compatibility', 'documentation']],
  ]);

  private readonly completedSteps: Map<string, Set<string>> = new Map();

  async enforceProcessOrder(
    process: string,
    currentStep: string
  ): Promise<void> {
    const required = this.requiredSteps.get(process);
    if (!required) return;

    const completed = this.completedSteps.get(process) || new Set();
    const currentIndex = required.indexOf(currentStep);

    if (currentIndex === -1) {
      throw new ProhibViolationError(
        'PROHIB-8',
        `Unknown step '${currentStep}' for process '${process}'`
      );
    }

    // Check all previous steps are completed
    for (let i = 0; i < currentIndex; i++) {
      if (!completed.has(required[i])) {
        throw new ProhibViolationError(
          'PROHIB-8',
          `Step '${currentStep}' cannot run before '${required[i]}' completes`
        );
      }
    }
  }

  markStepComplete(process: string, step: string): void {
    if (!this.completedSteps.has(process)) {
      this.completedSteps.set(process, new Set());
    }
    this.completedSteps.get(process)!.add(step);
  }

  async enforceApprovalRequired(
    operation: string,
    approvals: Approval[]
  ): Promise<void> {
    const requiredApprovers: Map<string, number> = new Map([
      ['production_deployment', 2],
      ['database_migration', 1],
      ['security_change', 2],
      ['infrastructure_change', 1],
    ]);

    const required = requiredApprovers.get(operation);
    if (!required) return;

    const validApprovals = approvals.filter(a =>
      a.isValid &&
      a.timestamp > Date.now() - 24 * 60 * 60 * 1000 // Within 24 hours
    );

    if (validApprovals.length < required) {
      throw new ProhibViolationError(
        'PROHIB-8',
        `Operation '${operation}' requires ${required} approvals, has ${validApprovals.length}`
      );
    }
  }

  async enforceDocumentationRequirement(
    changeType: string,
    documentation: Documentation
  ): Promise<void> {
    const requirements: Map<string, string[]> = new Map([
      ['api_change', ['endpoint_description', 'request_schema', 'response_schema', 'error_codes']],
      ['breaking_change', ['migration_guide', 'deprecation_notice', 'timeline']],
      ['new_feature', ['user_guide', 'api_reference', 'examples']],
    ]);

    const required = requirements.get(changeType);
    if (!required) return;

    const missing = required.filter(doc => !documentation[doc]);

    if (missing.length > 0) {
      throw new ProhibViolationError(
        'PROHIB-8',
        `Missing required documentation for ${changeType}: ${missing.join(', ')}`
      );
    }
  }
}
```

## Violation Handler

```typescript
export class ProhibViolationError extends Error {
  constructor(
    public readonly prohibId: string,
    public readonly message: string
  ) {
    super(`[${prohibId}] ${message}`);
    this.name = 'ProhibViolationError';
  }
}

export class ViolationHandler {
  async handleViolation(
    violation: ProhibViolation,
    context: ExecutionContext
  ): Promise<ViolationResponse> {
    // Step 1: Immediately halt execution
    await this.haltExecution(context);

    // Step 2: Log violation
    await this.logViolation(violation, context);

    // Step 3: Notify stakeholders
    await this.notifyViolation(violation, context);

    // Step 4: Store for audit
    await this.storeViolation(violation, context);

    // Step 5: Determine response
    return this.determineResponse(violation);
  }

  private async haltExecution(context: ExecutionContext): Promise<void> {
    context.status = 'halted';
    context.haltedAt = new Date().toISOString();

    // Stop all running agents in the phase
    for (const agentId of context.activeAgents) {
      await this.stopAgent(agentId);
    }
  }

  private async notifyViolation(
    violation: ProhibViolation,
    context: ExecutionContext
  ): Promise<void> {
    const notification = {
      level: violation.severity === 'critical' ? 'critical' : 'high',
      prohibId: violation.prohibId,
      message: violation.message,
      phase: context.phase,
      agent: context.currentAgent,
      timestamp: new Date().toISOString(),
    };

    // Store notification
    await this.storeMemory('coding/prohib/violations', notification);

    // Send alert
    console.error(`🚨 PROHIB VIOLATION: [${violation.prohibId}] ${violation.message}`);
  }

  private determineResponse(violation: ProhibViolation): ViolationResponse {
    return {
      action: 'halt',
      requiresManualIntervention: violation.severity === 'critical',
      recoveryOptions: violation.severity === 'critical'
        ? ['manual_review', 'rollback']
        : ['auto_remediate', 'manual_review', 'rollback'],
      nextSteps: this.getNextSteps(violation),
    };
  }
}
```

## Integration Hook

```typescript
// Pre-agent hook that runs PROHIB checks
export const prohibEnforcementHook = {
  event: 'pre_agent_execution',
  handler: async (context: HookContext) => {
    const enforcer = new ProhibEnforcementLayer();

    try {
      // Run all PROHIB checks
      await enforcer.enforceAll(context);
      return { proceed: true };
    } catch (error) {
      if (error instanceof ProhibViolationError) {
        const handler = new ViolationHandler();
        const response = await handler.handleViolation(
          { prohibId: error.prohibId, message: error.message, severity: 'critical' },
          context
        );

        return { proceed: false, response };
      }
      throw error;
    }
  },
};
```
