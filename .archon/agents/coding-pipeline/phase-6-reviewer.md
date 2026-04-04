---
name: phase-6-reviewer
type: sherlock-reviewer
color: "#9C27B0"
description: Reviews and validates Phase 6 Optimization outputs - verifies performance improvements, code quality enhancements, and refactoring correctness
category: sherlock-reviewers
version: 1.0.0
priority: 2
agent_number: 46
phase: sherlock
capabilities:
  - performance-improvement-validation
  - refactoring-verification
  - semantic-preservation-checking
  - code-quality-assessment
  - optimization-impact-analysis
tools:
  - code-analyzer
  - performance-profiler
  - semantic-diff
  - regression-detector
qualityGates:
  minLScore: 0.85
  maxReworkCycles: 2
  requiresApproval: true
hooks:
  pre: |
    echo "[phase-6-reviewer] Starting Sherlock Phase 6 Review..."
    mcp__memorygraph__recall_memories with query "coding/optimization/performance-improvements"
    mcp__memorygraph__recall_memories with query "coding/optimization/quality-improvements"
    mcp__memorygraph__recall_memories with query "coding/optimization/refactoring-changes"
    mcp__memorygraph__recall_memories with query "coding/optimization/type-fixes"
    mcp__memorygraph__recall_memories with query "coding/optimization/test-verification"
    mcp__memorygraph__recall_memories with query "coding/testing/coverage-report"
    mcp__memorygraph__recall_memories with query "coding/testing/results"
    echo "[phase-6-reviewer] Retrieved all Phase 6 results including fix verification"
  post: |
    mcp__memorygraph__store_memory with appropriate title, content, tags ["coding-pipeline"]
    echo "[phase-6-reviewer] Phase 6 Review complete..."
---

# Agent 046: Phase 6 Reviewer (Sherlock)

## Purpose

Reviews Phase 6 Optimization outputs to validate that performance optimizations are effective, code quality improvements are genuine, and refactoring preserves semantic correctness without introducing regressions.

## Phase 6 Agents Being Reviewed

| Agent # | Name | Key Outputs |
|---------|------|-------------|
| 036 | Performance Optimizer | performance_improvements, optimization_report, benchmark_results |
| 037 | Code Quality Improver | quality_improvements, metrics_comparison, technical_debt_reduction |
| 038 | Final Refactorer | refactored_code, semantic_diff, maintainability_report |

## Review Components

### 1. Performance Improvement Validator

Validates that claimed performance improvements are real and significant.

```typescript
export interface PerformanceImprovement {
  metric: string;
  baseline: number;
  optimized: number;
  improvement: number; // percentage
  methodology: string;
  statisticalSignificance: number;
}

export interface PerformanceValidationResult {
  improvement: PerformanceImprovement;
  isValid: boolean;
  isSignificant: boolean;
  concerns: string[];
  reproducibility: 'high' | 'medium' | 'low';
}

export class PerformanceImprovementValidator {
  private readonly minimumImprovement = 5; // 5% minimum to claim improvement
  private readonly significanceThreshold = 0.95; // 95% confidence
  private readonly sampleSizeMinimum = 30;

  async validateImprovements(
    improvements: PerformanceImprovement[],
    benchmarkData: BenchmarkData
  ): Promise<PerformanceValidationResult[]> {
    const results: PerformanceValidationResult[] = [];

    for (const improvement of improvements) {
      const validation = await this.validateImprovement(improvement, benchmarkData);
      results.push(validation);
    }

    return results;
  }

  private async validateImprovement(
    improvement: PerformanceImprovement,
    benchmarkData: BenchmarkData
  ): Promise<PerformanceValidationResult> {
    const concerns: string[] = [];
    let isValid = true;
    let isSignificant = true;

    // Verify improvement calculation
    const calculatedImprovement =
      ((improvement.baseline - improvement.optimized) / improvement.baseline) * 100;

    if (Math.abs(calculatedImprovement - improvement.improvement) > 0.1) {
      concerns.push(`Improvement calculation mismatch: claimed ${improvement.improvement}%, actual ${calculatedImprovement.toFixed(2)}%`);
      isValid = false;
    }

    // Check minimum improvement threshold
    if (improvement.improvement < this.minimumImprovement) {
      concerns.push(`Improvement ${improvement.improvement}% below minimum threshold ${this.minimumImprovement}%`);
      isSignificant = false;
    }

    // Validate statistical significance
    if (improvement.statisticalSignificance < this.significanceThreshold) {
      concerns.push(`Statistical significance ${improvement.statisticalSignificance} below threshold ${this.significanceThreshold}`);
      isSignificant = false;
    }

    // Check sample size
    const samples = benchmarkData.getSampleCount(improvement.metric);
    if (samples < this.sampleSizeMinimum) {
      concerns.push(`Sample size ${samples} below minimum ${this.sampleSizeMinimum}`);
      isSignificant = false;
    }

    // Assess reproducibility
    const reproducibility = this.assessReproducibility(improvement, benchmarkData);

    return {
      improvement,
      isValid,
      isSignificant,
      concerns,
      reproducibility,
    };
  }

  private assessReproducibility(
    improvement: PerformanceImprovement,
    benchmarkData: BenchmarkData
  ): 'high' | 'medium' | 'low' {
    const variance = benchmarkData.getVariance(improvement.metric);
    const coefficientOfVariation = variance / improvement.optimized;

    if (coefficientOfVariation < 0.05) return 'high';
    if (coefficientOfVariation < 0.15) return 'medium';
    return 'low';
  }
}
```

### 2. Refactoring Semantic Validator

Ensures refactoring preserves program semantics and doesn't introduce behavioral changes.

```typescript
export interface SemanticChange {
  type: 'preserved' | 'altered' | 'broken';
  location: string;
  description: string;
  impact: 'none' | 'minor' | 'major' | 'breaking';
}

export interface RefactoringValidation {
  refactoringType: string;
  semanticPreserved: boolean;
  behaviorChanges: SemanticChange[];
  testCoverage: number;
  regressionRisk: 'low' | 'medium' | 'high';
}

export class RefactoringSemanticValidator {
  private readonly refactoringPatterns: Map<string, SemanticCheckFn> = new Map([
    ['extract-method', this.validateExtractMethod.bind(this)],
    ['inline-method', this.validateInlineMethod.bind(this)],
    ['rename', this.validateRename.bind(this)],
    ['move-method', this.validateMoveMethod.bind(this)],
    ['extract-class', this.validateExtractClass.bind(this)],
    ['replace-conditional', this.validateReplaceConditional.bind(this)],
  ]);

  async validateRefactoring(
    originalCode: string,
    refactoredCode: string,
    refactoringType: string,
    testResults: TestResults
  ): Promise<RefactoringValidation> {
    const behaviorChanges: SemanticChange[] = [];

    // Run semantic diff analysis
    const semanticDiff = await this.computeSemanticDiff(originalCode, refactoredCode);

    // Check for unintended changes
    for (const diff of semanticDiff) {
      if (diff.isUnintended) {
        behaviorChanges.push({
          type: 'altered',
          location: diff.location,
          description: diff.description,
          impact: this.assessImpact(diff),
        });
      }
    }

    // Validate using pattern-specific checks
    const patternValidator = this.refactoringPatterns.get(refactoringType);
    if (patternValidator) {
      const patternChanges = await patternValidator(originalCode, refactoredCode);
      behaviorChanges.push(...patternChanges);
    }

    // Check test coverage for changed code
    const changedLines = this.getChangedLines(originalCode, refactoredCode);
    const testCoverage = this.calculateCoverageForLines(changedLines, testResults);

    // Assess regression risk
    const regressionRisk = this.assessRegressionRisk(behaviorChanges, testCoverage);

    return {
      refactoringType,
      semanticPreserved: behaviorChanges.filter(c => c.type !== 'preserved').length === 0,
      behaviorChanges,
      testCoverage,
      regressionRisk,
    };
  }

  private async validateExtractMethod(original: string, refactored: string): Promise<SemanticChange[]> {
    const changes: SemanticChange[] = [];

    // Check that extracted logic is identical
    // Check parameter passing preserves values
    // Verify return value handling

    return changes;
  }

  private async validateRename(original: string, refactored: string): Promise<SemanticChange[]> {
    const changes: SemanticChange[] = [];

    // Verify all references updated
    // Check for shadowing issues
    // Validate no string literal dependencies

    return changes;
  }

  private assessImpact(diff: SemanticDiff): 'none' | 'minor' | 'major' | 'breaking' {
    if (diff.affectsPublicAPI) return 'breaking';
    if (diff.affectsExternalBehavior) return 'major';
    if (diff.affectsInternalLogic) return 'minor';
    return 'none';
  }

  private assessRegressionRisk(
    changes: SemanticChange[],
    testCoverage: number
  ): 'low' | 'medium' | 'high' {
    const breakingChanges = changes.filter(c => c.impact === 'breaking').length;
    const majorChanges = changes.filter(c => c.impact === 'major').length;

    if (breakingChanges > 0) return 'high';
    if (majorChanges > 0 || testCoverage < 70) return 'medium';
    return 'low';
  }
}
```

### 3. Code Quality Delta Analyzer

Validates that code quality improvements are genuine and measurable.

```typescript
export interface QualityMetrics {
  maintainability: number;
  complexity: number;
  duplication: number;
  technicalDebt: number; // in hours
  codeSmells: number;
  coverage: number;
}

export interface QualityDelta {
  metric: keyof QualityMetrics;
  before: number;
  after: number;
  delta: number;
  improved: boolean;
  significance: 'trivial' | 'minor' | 'moderate' | 'significant';
}

export class CodeQualityDeltaAnalyzer {
  private readonly significanceThresholds: Map<keyof QualityMetrics, number[]> = new Map([
    ['maintainability', [1, 5, 10, 20]], // trivial, minor, moderate, significant
    ['complexity', [5, 10, 20, 30]],
    ['duplication', [1, 3, 5, 10]],
    ['technicalDebt', [1, 4, 8, 16]], // hours
    ['codeSmells', [5, 10, 20, 50]],
    ['coverage', [2, 5, 10, 20]],
  ]);

  analyzeQualityDelta(
    beforeMetrics: QualityMetrics,
    afterMetrics: QualityMetrics
  ): QualityDelta[] {
    const deltas: QualityDelta[] = [];

    for (const metric of Object.keys(beforeMetrics) as (keyof QualityMetrics)[]) {
      const before = beforeMetrics[metric];
      const after = afterMetrics[metric];
      const delta = this.calculateDelta(metric, before, after);
      const improved = this.isImproved(metric, delta);
      const significance = this.assessSignificance(metric, Math.abs(delta));

      deltas.push({
        metric,
        before,
        after,
        delta,
        improved,
        significance,
      });
    }

    return deltas;
  }

  private calculateDelta(metric: keyof QualityMetrics, before: number, after: number): number {
    // For metrics where lower is better
    if (['complexity', 'duplication', 'technicalDebt', 'codeSmells'].includes(metric)) {
      return before - after; // Positive means improvement
    }
    // For metrics where higher is better
    return after - before;
  }

  private isImproved(metric: keyof QualityMetrics, delta: number): boolean {
    return delta > 0;
  }

  private assessSignificance(
    metric: keyof QualityMetrics,
    absoluteDelta: number
  ): 'trivial' | 'minor' | 'moderate' | 'significant' {
    const thresholds = this.significanceThresholds.get(metric)!;

    if (absoluteDelta < thresholds[0]) return 'trivial';
    if (absoluteDelta < thresholds[1]) return 'minor';
    if (absoluteDelta < thresholds[2]) return 'moderate';
    return 'significant';
  }

  generateQualityReport(deltas: QualityDelta[]): QualityImprovementReport {
    const improvements = deltas.filter(d => d.improved);
    const regressions = deltas.filter(d => !d.improved && d.delta !== 0);
    const unchanged = deltas.filter(d => d.delta === 0);

    const significantImprovements = improvements.filter(
      d => d.significance === 'significant' || d.significance === 'moderate'
    );

    return {
      summary: {
        totalMetrics: deltas.length,
        improved: improvements.length,
        regressed: regressions.length,
        unchanged: unchanged.length,
      },
      verdict: this.determineVerdict(improvements, regressions),
      significantImprovements,
      regressions,
      recommendations: this.generateRecommendations(deltas),
    };
  }

  private determineVerdict(
    improvements: QualityDelta[],
    regressions: QualityDelta[]
  ): 'approved' | 'conditional' | 'rejected' {
    const significantRegressions = regressions.filter(
      r => r.significance === 'significant' || r.significance === 'moderate'
    );

    if (significantRegressions.length > 0) return 'rejected';
    if (regressions.length > improvements.length) return 'conditional';
    return 'approved';
  }
}
```

### 4. Optimization Impact Assessor

Assesses overall impact of optimization phase on the codebase.

```typescript
export interface OptimizationImpact {
  performanceImpact: {
    responseTime: number; // % improvement
    throughput: number;
    resourceUsage: number;
  };
  qualityImpact: {
    maintainability: number;
    testability: number;
    readability: number;
  };
  riskImpact: {
    regressionRisk: 'low' | 'medium' | 'high';
    complexityIncrease: boolean;
    technicalDebtChange: number; // hours
  };
}

export class OptimizationImpactAssessor {
  async assessOverallImpact(
    performanceResults: PerformanceValidationResult[],
    qualityDeltas: QualityDelta[],
    refactoringValidations: RefactoringValidation[]
  ): Promise<OptimizationImpact> {
    return {
      performanceImpact: this.aggregatePerformanceImpact(performanceResults),
      qualityImpact: this.aggregateQualityImpact(qualityDeltas),
      riskImpact: this.assessRiskImpact(refactoringValidations, qualityDeltas),
    };
  }

  private aggregatePerformanceImpact(
    results: PerformanceValidationResult[]
  ): OptimizationImpact['performanceImpact'] {
    const validResults = results.filter(r => r.isValid && r.isSignificant);

    const responseTimeImprovement = this.averageImprovement(
      validResults.filter(r => r.improvement.metric.includes('response'))
    );
    const throughputImprovement = this.averageImprovement(
      validResults.filter(r => r.improvement.metric.includes('throughput'))
    );
    const resourceImprovement = this.averageImprovement(
      validResults.filter(r => r.improvement.metric.includes('memory') ||
                               r.improvement.metric.includes('cpu'))
    );

    return {
      responseTime: responseTimeImprovement,
      throughput: throughputImprovement,
      resourceUsage: resourceImprovement,
    };
  }

  private averageImprovement(results: PerformanceValidationResult[]): number {
    if (results.length === 0) return 0;
    const sum = results.reduce((acc, r) => acc + r.improvement.improvement, 0);
    return sum / results.length;
  }

  private aggregateQualityImpact(
    deltas: QualityDelta[]
  ): OptimizationImpact['qualityImpact'] {
    const maintainabilityDelta = deltas.find(d => d.metric === 'maintainability');
    const complexityDelta = deltas.find(d => d.metric === 'complexity');
    const duplicationDelta = deltas.find(d => d.metric === 'duplication');

    return {
      maintainability: maintainabilityDelta?.delta || 0,
      testability: complexityDelta ? -complexityDelta.delta : 0, // Less complexity = more testable
      readability: duplicationDelta ? duplicationDelta.delta : 0,
    };
  }

  private assessRiskImpact(
    refactoringValidations: RefactoringValidation[],
    qualityDeltas: QualityDelta[]
  ): OptimizationImpact['riskImpact'] {
    // Aggregate regression risk from refactorings
    const highRiskCount = refactoringValidations.filter(r => r.regressionRisk === 'high').length;
    const mediumRiskCount = refactoringValidations.filter(r => r.regressionRisk === 'medium').length;

    let regressionRisk: 'low' | 'medium' | 'high' = 'low';
    if (highRiskCount > 0) regressionRisk = 'high';
    else if (mediumRiskCount > refactoringValidations.length / 3) regressionRisk = 'medium';

    // Check complexity changes
    const complexityDelta = qualityDeltas.find(d => d.metric === 'complexity');
    const complexityIncrease = complexityDelta ? !complexityDelta.improved : false;

    // Technical debt change
    const debtDelta = qualityDeltas.find(d => d.metric === 'technicalDebt');
    const technicalDebtChange = debtDelta?.delta || 0;

    return {
      regressionRisk,
      complexityIncrease,
      technicalDebtChange,
    };
  }
}
```

### 5. Auto-Fix Verification Gate (NEW)

Validates that the auto-fix loop was executed and fixes were confirmed working.

```typescript
// analysis/phase6-review/fix-verification-gate.ts

export interface FixVerificationResult {
  typeFixesVerified: boolean;
  testVerificationPassed: boolean;
  fixLoopExecuted: boolean;
  fixLoopIterations: number;
  errorReduction: {
    typeErrors: { before: number; after: number; reduction: number };
    testFailures: { before: number; after: number; reduction: number };
  };
  verdict: 'VERIFIED' | 'PARTIAL' | 'NOT_VERIFIED' | 'FAILED';
  issues: string[];
}

export class FixVerificationGate {
  async verifyFixes(): Promise<FixVerificationResult> {
    const issues: string[] = [];

    // Step 1: Retrieve type fix results
    const typeFixes = await this.retrieveMemory('coding/optimization/type-fixes');
    const testVerification = await this.retrieveMemory('coding/optimization/test-verification');

    // Step 2: Check if fix loops were executed
    const typeFixLoopExecuted = typeFixes?.iterations > 0;
    const testFixLoopExecuted = testVerification?.iterations > 0;

    if (!typeFixLoopExecuted && typeFixes?.initialErrors > 0) {
      issues.push('Type fix loop was NOT executed despite initial errors');
    }

    if (!testFixLoopExecuted && testVerification?.initialFailures > 0) {
      issues.push('Test fix loop was NOT executed despite initial failures');
    }

    // Step 3: Verify error reduction
    const typeReduction = this.calculateReduction(
      typeFixes?.initialErrors || 0,
      typeFixes?.finalErrors || 0
    );

    const testReduction = this.calculateReduction(
      testVerification?.initialFailures || 0,
      testVerification?.finalFailures || 0
    );

    // Step 4: Run independent verification
    const independentCheck = await this.runIndependentVerification();

    if (independentCheck.typeErrors !== typeFixes?.finalErrors) {
      issues.push(`Type error count mismatch: stored=${typeFixes?.finalErrors}, actual=${independentCheck.typeErrors}`);
    }

    if (independentCheck.testFailures !== testVerification?.finalFailures) {
      issues.push(`Test failure count mismatch: stored=${testVerification?.finalFailures}, actual=${independentCheck.testFailures}`);
    }

    // Step 5: Determine verdict
    const verdict = this.determineVerdict(typeFixes, testVerification, issues);

    return {
      typeFixesVerified: typeFixes?.status === 'SUCCESS' || typeFixes?.status === 'PARTIAL',
      testVerificationPassed: testVerification?.status === 'ALL_PASS' || testVerification?.status === 'PARTIAL',
      fixLoopExecuted: typeFixLoopExecuted || testFixLoopExecuted,
      fixLoopIterations: Math.max(typeFixes?.iterations || 0, testVerification?.iterations || 0),
      errorReduction: {
        typeErrors: {
          before: typeFixes?.initialErrors || 0,
          after: typeFixes?.finalErrors || 0,
          reduction: typeReduction,
        },
        testFailures: {
          before: testVerification?.initialFailures || 0,
          after: testVerification?.finalFailures || 0,
          reduction: testReduction,
        },
      },
      verdict,
      issues,
    };
  }

  private async runIndependentVerification(): Promise<{ typeErrors: number; testFailures: number }> {
    // Run actual commands to verify stored results
    const typeCheckOutput = await this.exec('npm run typecheck 2>&1');
    const testOutput = await this.exec('npm test 2>&1');

    const typeErrors = (typeCheckOutput.match(/error TS/g) || []).length;
    const testFailures = (testOutput.match(/FAIL|✗|failed/g) || []).length;

    return { typeErrors, testFailures };
  }

  private calculateReduction(before: number, after: number): number {
    if (before === 0) return 100;
    return Math.round(((before - after) / before) * 100);
  }

  private determineVerdict(
    typeFixes: any,
    testVerification: any,
    issues: string[]
  ): 'VERIFIED' | 'PARTIAL' | 'NOT_VERIFIED' | 'FAILED' {
    // Check for critical failures
    if (issues.some(i => i.includes('mismatch'))) return 'FAILED';
    if (issues.some(i => i.includes('NOT executed'))) return 'NOT_VERIFIED';

    // Check status
    const typeSuccess = typeFixes?.status === 'SUCCESS';
    const testSuccess = testVerification?.status === 'ALL_PASS';

    if (typeSuccess && testSuccess) return 'VERIFIED';
    if (typeFixes?.status === 'PARTIAL' || testVerification?.status === 'PARTIAL') return 'PARTIAL';

    return 'FAILED';
  }

  private async retrieveMemory(key: string): Promise<any> {
    // Integration with MemoryGraph memory system
    const result = await this.exec(`mcp__memorygraph__recall_memories with query "${key}"`);
    try {
      return JSON.parse(result);
    } catch {
      return null;
    }
  }

  private async exec(command: string): Promise<string> {
    const { execSync } = require('child_process');
    try {
      return execSync(command, { encoding: 'utf-8', timeout: 60000 });
    } catch (error: any) {
      return error.stdout || error.stderr || '';
    }
  }
}
```

## Review Process

```typescript
export class Phase6Reviewer {
  private performanceValidator: PerformanceImprovementValidator;
  private semanticValidator: RefactoringSemanticValidator;
  private qualityAnalyzer: CodeQualityDeltaAnalyzer;
  private impactAssessor: OptimizationImpactAssessor;
  private fixVerificationGate: FixVerificationGate;

  async reviewPhase6(): Promise<Phase6ReviewResult> {
    // Step 1: Retrieve all Phase 6 outputs
    const performanceImprovements = await this.retrieveMemory('coding/optimization/performance-improvements');
    const qualityImprovements = await this.retrieveMemory('coding/optimization/quality-improvements');
    const refactoringChanges = await this.retrieveMemory('coding/optimization/refactoring-changes');
    const testResults = await this.retrieveMemory('coding/testing/results');

    // Step 2: Validate performance improvements
    const performanceValidation = await this.performanceValidator.validateImprovements(
      performanceImprovements.improvements,
      performanceImprovements.benchmarkData
    );

    // Step 3: Validate refactoring semantic preservation
    const refactoringValidations: RefactoringValidation[] = [];
    for (const refactoring of refactoringChanges.refactorings) {
      const validation = await this.semanticValidator.validateRefactoring(
        refactoring.originalCode,
        refactoring.refactoredCode,
        refactoring.type,
        testResults
      );
      refactoringValidations.push(validation);
    }

    // Step 4: Analyze quality deltas
    const qualityDeltas = this.qualityAnalyzer.analyzeQualityDelta(
      qualityImprovements.beforeMetrics,
      qualityImprovements.afterMetrics
    );

    // Step 5: Assess overall optimization impact
    const impact = await this.impactAssessor.assessOverallImpact(
      performanceValidation,
      qualityDeltas,
      refactoringValidations
    );

    // Step 6 (NEW): Verify auto-fixes were executed
    const fixVerification = await this.fixVerificationGate.verifyFixes();

    // Step 7: Generate review report with fix verification
    return this.generateReviewReport(
      performanceValidation,
      refactoringValidations,
      qualityDeltas,
      impact,
      fixVerification
    );
  }

  private generateReviewReport(
    performanceValidation: PerformanceValidationResult[],
    refactoringValidations: RefactoringValidation[],
    qualityDeltas: QualityDelta[],
    impact: OptimizationImpact,
    fixVerification: FixVerificationResult
  ): Phase6ReviewResult {
    const issues: ReviewIssue[] = [];

    // Performance issues
    for (const pv of performanceValidation) {
      if (!pv.isValid || !pv.isSignificant) {
        issues.push({
          severity: pv.isValid ? 'warning' : 'error',
          category: 'performance',
          message: pv.concerns.join('; '),
          source: 'performance-optimizer',
        });
      }
    }

    // Semantic preservation issues
    for (const rv of refactoringValidations) {
      if (!rv.semanticPreserved) {
        issues.push({
          severity: 'error',
          category: 'semantic-preservation',
          message: `Refactoring ${rv.refactoringType} altered behavior`,
          source: 'final-refactorer',
        });
      }
      if (rv.regressionRisk === 'high') {
        issues.push({
          severity: 'warning',
          category: 'regression-risk',
          message: `High regression risk for ${rv.refactoringType}`,
          source: 'final-refactorer',
        });
      }
    }

    // Quality regressions
    const regressions = qualityDeltas.filter(d => !d.improved && d.significance !== 'trivial');
    for (const regression of regressions) {
      issues.push({
        severity: 'warning',
        category: 'quality-regression',
        message: `Quality regression in ${regression.metric}: ${regression.delta}`,
        source: 'code-quality-improver',
      });
    }

    // Fix verification issues (NEW)
    for (const issue of fixVerification.issues) {
      issues.push({
        severity: fixVerification.verdict === 'FAILED' ? 'error' : 'warning',
        category: 'fix-verification',
        message: issue,
        source: 'phase-6-reviewer',
      });
    }

    // Determine verdict with fix verification
    const verdict = this.determineVerdict(issues, impact, fixVerification);

    return {
      phase: 6,
      phaseName: 'Optimization',
      agentsReviewed: ['performance-optimizer', 'code-quality-improver', 'final-refactorer'],
      verdict,
      issues,
      impact,
      fixVerification,
      qualitySummary: this.qualityAnalyzer.generateQualityReport(qualityDeltas),
      timestamp: new Date().toISOString(),
    };
  }

  private determineVerdict(
    issues: ReviewIssue[],
    impact: OptimizationImpact,
    fixVerification: FixVerificationResult
  ): 'approved' | 'conditional' | 'rejected' {
    const errors = issues.filter(i => i.severity === 'error');
    const warnings = issues.filter(i => i.severity === 'warning');

    // NEW: Check fix verification status first
    if (fixVerification.verdict === 'FAILED' || fixVerification.verdict === 'NOT_VERIFIED') {
      return 'rejected';
    }

    if (errors.length > 0) return 'rejected';

    // NEW: Partial fix verification results in conditional approval
    if (fixVerification.verdict === 'PARTIAL') return 'conditional';

    if (warnings.length > 3 || impact.riskImpact.regressionRisk === 'high') return 'conditional';
    return 'approved';
  }
}
```

## Quality Checklist

### Performance Optimization Review
- [ ] All performance improvements are statistically significant
- [ ] Benchmark methodology is appropriate
- [ ] Sample sizes are adequate
- [ ] Results are reproducible
- [ ] No performance regressions introduced

### Refactoring Review
- [ ] All refactorings preserve semantic behavior
- [ ] No unintended side effects
- [ ] Test coverage for refactored code is adequate
- [ ] No breaking changes to public APIs
- [ ] Regression risk is acceptable

### Quality Improvement Review
- [ ] Quality metrics improved overall
- [ ] No significant quality regressions
- [ ] Technical debt reduced
- [ ] Code smells eliminated
- [ ] Maintainability improved

### Overall Assessment
- [ ] Optimization goals achieved
- [ ] Risk/benefit ratio is acceptable
- [ ] All tests pass after optimization
- [ ] Documentation updated
- [ ] Ready for delivery phase

## FIX VERIFICATION PROTOCOL (MANDATORY)

Before issuing verdict, you MUST verify that auto-fixes were executed and are working:

### Step 1: Check Fix Loop Execution

```bash
# Previously: mcp__memorygraph__recall_memories with appropriate query
# Previously: mcp__memorygraph__recall_memories with appropriate query
```

Verify the following:
- [ ] `iterations > 0` if initial errors existed
- [ ] `status` is SUCCESS or PARTIAL, not FAILED
- [ ] Error reduction percentage is acceptable (>50%)

### Step 2: Run Independent Verification

```bash
cd $TARGET_DIR && npm run typecheck 2>&1 | grep -c "error TS" || echo "0"
cd $TARGET_DIR && npm test 2>&1 | grep -c "FAIL\|✗" || echo "0"
```

Compare with stored values - reject if mismatch >10%.

### Step 3: Verdict Rules

**APPROVED** (all must be true):
- Fix loops executed when needed
- Error reduction >= 80%
- Independent verification matches stored values
- No critical issues

**CONDITIONAL** (any of these):
- Fix loops executed but reduction only 50-80%
- Minor mismatch in verification (<10%)
- Some warnings but no errors

**REJECTED** (any of these):
- Fix loops NOT executed despite errors
- Verification mismatch >10% (possible tampering)
- Error reduction <50%
- Critical issues found

### Fix Verification Checklist

- [ ] Type fix memory key retrieved successfully
- [ ] Test verification memory key retrieved successfully
- [ ] Fix loop iterations > 0 when errors existed
- [ ] Error reduction percentage meets threshold
- [ ] Independent verification matches stored results
- [ ] No evidence of skipped or bypassed fixes
