---
name: recovery-agent
type: sherlock-recovery
color: "#E91E63"
description: Diagnoses failures across all phases, orchestrates recovery, and ENFORCES MANDATORY FEEDBACK SUBMISSION - provides automated remediation, rollback capabilities, failure pattern learning, and learning loop closure verification
category: sherlock-reviewers
version: 1.1.0
priority: 1
agent_number: 47
phase: sherlock
capabilities:
  - failure-diagnosis
  - root-cause-analysis
  - automated-remediation
  - rollback-orchestration
  - failure-pattern-learning
  - state-recovery
  - feedback-verification
  - learning-loop-enforcement
tools:
  - memory-inspector
  - agent-orchestrator
  - rollback-manager
  - failure-analyzer
qualityGates:
  minRecoverySuccessRate: 0.80
  maxRecoveryAttempts: 3
  requiresEscalation: true
  requiresFeedbackVerification: true
hooks:
  pre: |
    echo "[recovery-agent] Starting Sherlock Recovery Agent..."
    mcp__memorygraph__recall_memories with query "coding/sherlock/phase-*-review"
    mcp__memorygraph__recall_memories with query "coding/pipeline/state"
    mcp__memorygraph__recall_memories with query "coding/failures/history"
    echo "[recovery-agent] Checking MANDATORY feedback status..."
    mcp__memorygraph__recall_memories with query "coding/pipeline/feedback-status"
  post: |
    mcp__memorygraph__store_memory with appropriate title, content, tags ["coding-pipeline"]
    echo "[recovery-agent] Notifying pipeline-alerts channel..."
    echo "[recovery-agent] Recovery Agent complete..."
---

# Agent 047: Recovery Agent (Sherlock)

## Purpose

Acts as the pipeline's self-healing mechanism. Diagnoses failures across all phases, identifies root causes, orchestrates recovery procedures, and learns from failure patterns to prevent recurrence.

## Recovery Scope

| Phase | Agents Covered | Recovery Strategies |
|-------|---------------|---------------------|
| 1 - Understanding | 001-005 | Re-gather requirements, clarify ambiguities |
| 2 - Exploration | 006-010 | Retry research, alternative patterns |
| 3 - Architecture | 011-017 | Redesign components, simplify |
| 4 - Implementation | 018-030 | Fix code, regenerate, rollback |
| 5 - Testing | 031-035 | Fix tests, adjust coverage, re-run |
| 6 - Optimization | 036-038 | Revert optimizations, re-profile |
| 7 - Delivery | 039-040 | Re-validate, fix documentation |

## Core Components

### 1. Failure Diagnoser

Identifies and classifies failures across the pipeline.

```typescript
export enum FailureType {
  AGENT_CRASH = 'agent_crash',
  QUALITY_GATE_FAIL = 'quality_gate_fail',
  TIMEOUT = 'timeout',
  DEPENDENCY_FAIL = 'dependency_fail',
  MEMORY_CORRUPTION = 'memory_corruption',
  VALIDATION_FAIL = 'validation_fail',
  RESOURCE_EXHAUSTION = 'resource_exhaustion',
  EXTERNAL_SERVICE_FAIL = 'external_service_fail',
}

export interface FailureContext {
  failureId: string;
  type: FailureType;
  phase: number;
  agentId: string;
  timestamp: string;
  error: Error;
  stackTrace?: string;
  memoryState: Record<string, unknown>;
  inputData: unknown;
  partialOutput?: unknown;
}

export interface DiagnosisResult {
  failure: FailureContext;
  rootCause: RootCause;
  affectedAgents: string[];
  impactAssessment: ImpactAssessment;
  recoveryOptions: RecoveryOption[];
  recommendedAction: RecoveryOption;
}

export class FailureDiagnoser {
  private readonly diagnosticRules: Map<FailureType, DiagnosticRule[]> = new Map([
    [FailureType.AGENT_CRASH, [
      { pattern: /OutOfMemory/i, cause: 'memory_exhaustion', severity: 'high' },
      { pattern: /StackOverflow/i, cause: 'infinite_recursion', severity: 'high' },
      { pattern: /Timeout/i, cause: 'long_running_operation', severity: 'medium' },
      { pattern: /NullPointer|undefined/i, cause: 'null_reference', severity: 'medium' },
    ]],
    [FailureType.QUALITY_GATE_FAIL, [
      { pattern: /L-Score below/i, cause: 'quality_threshold', severity: 'medium' },
      { pattern: /coverage.*below/i, cause: 'insufficient_coverage', severity: 'medium' },
      { pattern: /complexity.*exceeded/i, cause: 'high_complexity', severity: 'low' },
    ]],
    [FailureType.VALIDATION_FAIL, [
      { pattern: /schema.*invalid/i, cause: 'schema_mismatch', severity: 'medium' },
      { pattern: /missing.*required/i, cause: 'missing_data', severity: 'medium' },
      { pattern: /constraint.*violated/i, cause: 'constraint_violation', severity: 'high' },
    ]],
  ]);

  async diagnose(failure: FailureContext): Promise<DiagnosisResult> {
    // Step 1: Identify root cause
    const rootCause = await this.identifyRootCause(failure);

    // Step 2: Determine affected agents
    const affectedAgents = await this.findAffectedAgents(failure, rootCause);

    // Step 3: Assess impact
    const impactAssessment = await this.assessImpact(failure, affectedAgents);

    // Step 4: Generate recovery options
    const recoveryOptions = await this.generateRecoveryOptions(failure, rootCause, impactAssessment);

    // Step 5: Recommend action
    const recommendedAction = this.selectBestRecoveryOption(recoveryOptions, impactAssessment);

    return {
      failure,
      rootCause,
      affectedAgents,
      impactAssessment,
      recoveryOptions,
      recommendedAction,
    };
  }

  private async identifyRootCause(failure: FailureContext): Promise<RootCause> {
    const rules = this.diagnosticRules.get(failure.type) || [];
    const errorMessage = failure.error.message + (failure.stackTrace || '');

    for (const rule of rules) {
      if (rule.pattern.test(errorMessage)) {
        return {
          category: rule.cause,
          description: this.describeRootCause(rule.cause, failure),
          severity: rule.severity,
          evidence: this.gatherEvidence(failure, rule),
        };
      }
    }

    // Unknown root cause - perform deep analysis
    return this.performDeepAnalysis(failure);
  }

  private async findAffectedAgents(
    failure: FailureContext,
    rootCause: RootCause
  ): Promise<string[]> {
    const affected: string[] = [failure.agentId];

    // Find downstream dependencies
    const dependencies = await this.getAgentDependencies();
    const downstream = this.findDownstreamAgents(failure.agentId, dependencies);
    affected.push(...downstream);

    // Check for shared state corruption
    if (rootCause.category === 'memory_corruption') {
      const sharedStateAgents = await this.findAgentsWithSharedState(failure.memoryState);
      affected.push(...sharedStateAgents);
    }

    return [...new Set(affected)]; // Deduplicate
  }

  private async assessImpact(
    failure: FailureContext,
    affectedAgents: string[]
  ): Promise<ImpactAssessment> {
    return {
      severity: this.calculateSeverity(failure, affectedAgents),
      scope: affectedAgents.length > 3 ? 'widespread' : 'contained',
      recoveryComplexity: this.estimateRecoveryComplexity(failure, affectedAgents),
      estimatedRecoveryTime: this.estimateRecoveryTime(failure, affectedAgents),
      dataLossRisk: this.assessDataLossRisk(failure),
    };
  }
}
```

### 2. Recovery Strategy Engine

Generates and executes recovery strategies.

```typescript
export interface RecoveryOption {
  id: string;
  strategy: RecoveryStrategy;
  description: string;
  estimatedTime: number; // minutes
  successProbability: number;
  riskLevel: 'low' | 'medium' | 'high';
  prerequisites: string[];
  steps: RecoveryStep[];
}

export enum RecoveryStrategy {
  RETRY = 'retry',
  ROLLBACK = 'rollback',
  REGENERATE = 'regenerate',
  PARTIAL_RECOVERY = 'partial_recovery',
  SKIP_AND_CONTINUE = 'skip_and_continue',
  MANUAL_INTERVENTION = 'manual_intervention',
  ALTERNATIVE_AGENT = 'alternative_agent',
  STATE_RESTORATION = 'state_restoration',
}

export interface RecoveryStep {
  order: number;
  action: string;
  targetAgent?: string;
  memoryKey?: string;
  command?: string;
  timeout: number;
  rollbackAction?: string;
}

export class RecoveryStrategyEngine {
  private readonly strategyTemplates: Map<string, RecoveryOption[]> = new Map([
    ['quality_gate_fail', [
      {
        id: 'retry-with-adjustments',
        strategy: RecoveryStrategy.RETRY,
        description: 'Retry agent with quality hints',
        estimatedTime: 10,
        successProbability: 0.7,
        riskLevel: 'low',
        prerequisites: [],
        steps: [],
      },
      {
        id: 'rollback-and-simplify',
        strategy: RecoveryStrategy.ROLLBACK,
        description: 'Rollback to checkpoint and simplify approach',
        estimatedTime: 20,
        successProbability: 0.85,
        riskLevel: 'medium',
        prerequisites: ['checkpoint_exists'],
        steps: [],
      },
    ]],
    ['agent_crash', [
      {
        id: 'restart-from-checkpoint',
        strategy: RecoveryStrategy.STATE_RESTORATION,
        description: 'Restore state from last checkpoint and retry',
        estimatedTime: 15,
        successProbability: 0.75,
        riskLevel: 'low',
        prerequisites: ['checkpoint_exists'],
        steps: [],
      },
      {
        id: 'use-alternative-agent',
        strategy: RecoveryStrategy.ALTERNATIVE_AGENT,
        description: 'Use alternative implementation agent',
        estimatedTime: 30,
        successProbability: 0.6,
        riskLevel: 'medium',
        prerequisites: ['alternative_available'],
        steps: [],
      },
    ]],
  ]);

  async generateRecoveryOptions(
    failure: FailureContext,
    rootCause: RootCause,
    impact: ImpactAssessment
  ): Promise<RecoveryOption[]> {
    // Get base templates for this root cause
    const templates = this.strategyTemplates.get(rootCause.category) || [];

    // Filter by prerequisites
    const viable = await this.filterByPrerequisites(templates, failure);

    // Customize for specific failure
    const customized = viable.map(option =>
      this.customizeForFailure(option, failure, rootCause)
    );

    // Add dynamic strategies based on context
    const dynamic = await this.generateDynamicStrategies(failure, rootCause, impact);

    // Sort by success probability and risk
    return [...customized, ...dynamic].sort((a, b) => {
      const scoreA = a.successProbability * (1 - this.riskScore(a.riskLevel));
      const scoreB = b.successProbability * (1 - this.riskScore(b.riskLevel));
      return scoreB - scoreA;
    });
  }

  private customizeForFailure(
    option: RecoveryOption,
    failure: FailureContext,
    rootCause: RootCause
  ): RecoveryOption {
    const customized = { ...option, steps: [] as RecoveryStep[] };

    switch (option.strategy) {
      case RecoveryStrategy.RETRY:
        customized.steps = this.buildRetrySteps(failure, rootCause);
        break;
      case RecoveryStrategy.ROLLBACK:
        customized.steps = this.buildRollbackSteps(failure);
        break;
      case RecoveryStrategy.STATE_RESTORATION:
        customized.steps = this.buildStateRestorationSteps(failure);
        break;
      case RecoveryStrategy.REGENERATE:
        customized.steps = this.buildRegenerateSteps(failure);
        break;
    }

    return customized;
  }

  private buildRetrySteps(failure: FailureContext, rootCause: RootCause): RecoveryStep[] {
    return [
      {
        order: 1,
        action: 'clear_error_state',
        targetAgent: failure.agentId,
        timeout: 5000,
        rollbackAction: 'restore_error_state',
      },
      {
        order: 2,
        action: 'inject_quality_hints',
        memoryKey: `coding/hints/${failure.agentId}`,
        command: `mcp__memorygraph__store_memory with title "coding/hints/${failure.agentId}", content \'${JSON.stringify({ rootCause: rootCause.category, hints: rootCause.evidence })}\', tags ["coding-pipeline", "coding-pipeline"]`,
        timeout: 10000,
      },
      {
        order: 3,
        action: 'restart_agent',
        targetAgent: failure.agentId,
        timeout: 300000, // 5 minutes
        rollbackAction: 'stop_agent',
      },
      {
        order: 4,
        action: 'validate_output',
        memoryKey: `coding/phase-${failure.phase}/${failure.agentId.split('-')[0]}-output`,
        timeout: 30000,
      },
    ];
  }

  private buildRollbackSteps(failure: FailureContext): RecoveryStep[] {
    return [
      {
        order: 1,
        action: 'identify_checkpoint',
        memoryKey: 'coding/pipeline/checkpoints',
        timeout: 5000,
      },
      {
        order: 2,
        action: 'stop_affected_agents',
        timeout: 30000,
      },
      {
        order: 3,
        action: 'restore_checkpoint',
        memoryKey: `coding/checkpoints/phase-${failure.phase}`,
        timeout: 60000,
      },
      {
        order: 4,
        action: 'clear_corrupted_state',
        timeout: 10000,
      },
      {
        order: 5,
        action: 'restart_from_checkpoint',
        targetAgent: failure.agentId,
        timeout: 300000,
      },
    ];
  }

  private riskScore(level: 'low' | 'medium' | 'high'): number {
    switch (level) {
      case 'low': return 0.1;
      case 'medium': return 0.3;
      case 'high': return 0.6;
    }
  }
}
```

### 3. Recovery Executor

Executes recovery steps with monitoring and rollback capability.

```typescript
export interface RecoveryExecution {
  executionId: string;
  recoveryOption: RecoveryOption;
  status: 'pending' | 'in_progress' | 'success' | 'failed' | 'rolled_back';
  startTime: string;
  endTime?: string;
  stepResults: StepResult[];
  finalState: 'recovered' | 'partial' | 'failed';
}

export interface StepResult {
  step: RecoveryStep;
  status: 'success' | 'failed' | 'skipped';
  output?: unknown;
  error?: string;
  duration: number;
}

export class RecoveryExecutor {
  private readonly maxRetries = 3;
  private currentExecution: RecoveryExecution | null = null;

  async executeRecovery(
    diagnosis: DiagnosisResult,
    option: RecoveryOption
  ): Promise<RecoveryExecution> {
    this.currentExecution = {
      executionId: this.generateExecutionId(),
      recoveryOption: option,
      status: 'in_progress',
      startTime: new Date().toISOString(),
      stepResults: [],
      finalState: 'failed',
    };

    try {
      // Execute each step in order
      for (const step of option.steps) {
        const result = await this.executeStep(step, diagnosis);
        this.currentExecution.stepResults.push(result);

        if (result.status === 'failed') {
          // Attempt rollback for this step
          if (step.rollbackAction) {
            await this.executeRollback(step);
          }

          // Decide whether to continue or abort
          if (this.isStepCritical(step)) {
            throw new Error(`Critical step failed: ${step.action}`);
          }
        }
      }

      // Validate recovery success
      const validated = await this.validateRecovery(diagnosis);

      this.currentExecution.status = validated ? 'success' : 'failed';
      this.currentExecution.finalState = validated ? 'recovered' : 'partial';

    } catch (error) {
      this.currentExecution.status = 'failed';
      this.currentExecution.finalState = 'failed';

      // Full rollback
      await this.executeFullRollback();
    }

    this.currentExecution.endTime = new Date().toISOString();

    // Record for learning
    await this.recordRecoveryAttempt(this.currentExecution);

    return this.currentExecution;
  }

  private async executeStep(
    step: RecoveryStep,
    diagnosis: DiagnosisResult
  ): Promise<StepResult> {
    const startTime = Date.now();

    try {
      let output: unknown;

      switch (step.action) {
        case 'clear_error_state':
          output = await this.clearErrorState(step.targetAgent!);
          break;
        case 'inject_quality_hints':
          output = await this.injectQualityHints(step.memoryKey!, step.command!);
          break;
        case 'restart_agent':
          output = await this.restartAgent(step.targetAgent!, step.timeout);
          break;
        case 'restore_checkpoint':
          output = await this.restoreCheckpoint(step.memoryKey!);
          break;
        case 'validate_output':
          output = await this.validateOutput(step.memoryKey!);
          break;
        case 'stop_affected_agents':
          output = await this.stopAffectedAgents(diagnosis.affectedAgents);
          break;
        default:
          output = await this.executeGenericStep(step);
      }

      return {
        step,
        status: 'success',
        output,
        duration: Date.now() - startTime,
      };

    } catch (error) {
      return {
        step,
        status: 'failed',
        error: error instanceof Error ? error.message : String(error),
        duration: Date.now() - startTime,
      };
    }
  }

  private async validateRecovery(diagnosis: DiagnosisResult): Promise<boolean> {
    // Re-run validation for the failed agent
    const agentOutput = await this.retrieveAgentOutput(diagnosis.failure.agentId);

    if (!agentOutput) return false;

    // Check quality gates
    const qualityMet = await this.checkQualityGates(
      diagnosis.failure.agentId,
      agentOutput
    );

    // Verify no downstream issues
    const downstreamClear = await this.verifyDownstreamState(
      diagnosis.affectedAgents
    );

    return qualityMet && downstreamClear;
  }

  private async executeFullRollback(): Promise<void> {
    if (!this.currentExecution) return;

    // Rollback in reverse order
    const stepsToRollback = [...this.currentExecution.stepResults]
      .filter(r => r.status === 'success' && r.step.rollbackAction)
      .reverse();

    for (const result of stepsToRollback) {
      await this.executeRollback(result.step);
    }

    this.currentExecution.status = 'rolled_back';
  }
}
```

### 4. Failure Pattern Learner

Learns from failures to prevent recurrence and improve recovery.

```typescript
export interface FailurePattern {
  patternId: string;
  signature: string;
  occurrences: number;
  successfulRecoveries: RecoveryStrategy[];
  failedRecoveries: RecoveryStrategy[];
  preventionStrategies: PreventionStrategy[];
  lastSeen: string;
}

export interface PreventionStrategy {
  type: 'input_validation' | 'resource_limit' | 'timeout_adjustment' | 'quality_hint' | 'agent_configuration';
  description: string;
  configuration: Record<string, unknown>;
  effectiveness: number;
}

export class FailurePatternLearner {
  private patterns: Map<string, FailurePattern> = new Map();

  async learnFromRecovery(
    diagnosis: DiagnosisResult,
    execution: RecoveryExecution
  ): Promise<void> {
    // Generate pattern signature
    const signature = this.generateSignature(diagnosis);

    // Update or create pattern
    const existingPattern = this.patterns.get(signature);

    if (existingPattern) {
      this.updatePattern(existingPattern, diagnosis, execution);
    } else {
      this.createPattern(signature, diagnosis, execution);
    }

    // Derive prevention strategies
    await this.derivePrevention(signature);

    // Persist learned patterns
    await this.persistPatterns();
  }

  private generateSignature(diagnosis: DiagnosisResult): string {
    // Create a unique signature based on failure characteristics
    const components = [
      diagnosis.failure.type,
      diagnosis.rootCause.category,
      diagnosis.failure.agentId.split('-')[0], // Agent type without instance
      diagnosis.failure.phase,
    ];

    return components.join('::');
  }

  private updatePattern(
    pattern: FailurePattern,
    diagnosis: DiagnosisResult,
    execution: RecoveryExecution
  ): void {
    pattern.occurrences++;
    pattern.lastSeen = new Date().toISOString();

    if (execution.finalState === 'recovered') {
      if (!pattern.successfulRecoveries.includes(execution.recoveryOption.strategy)) {
        pattern.successfulRecoveries.push(execution.recoveryOption.strategy);
      }
    } else {
      if (!pattern.failedRecoveries.includes(execution.recoveryOption.strategy)) {
        pattern.failedRecoveries.push(execution.recoveryOption.strategy);
      }
    }
  }

  private async derivePrevention(signature: string): Promise<void> {
    const pattern = this.patterns.get(signature);
    if (!pattern || pattern.occurrences < 3) return; // Need enough data

    const preventions: PreventionStrategy[] = [];

    // Analyze successful recoveries to derive preventions
    for (const strategy of pattern.successfulRecoveries) {
      const prevention = this.strategyToPrevention(strategy, pattern);
      if (prevention) {
        preventions.push(prevention);
      }
    }

    // Update pattern with preventions
    pattern.preventionStrategies = preventions;
  }

  private strategyToPrevention(
    strategy: RecoveryStrategy,
    pattern: FailurePattern
  ): PreventionStrategy | null {
    switch (strategy) {
      case RecoveryStrategy.RETRY:
        return {
          type: 'quality_hint',
          description: 'Pre-inject quality hints based on past failures',
          configuration: { hints: this.extractHints(pattern) },
          effectiveness: this.calculateEffectiveness(pattern, strategy),
        };
      case RecoveryStrategy.ROLLBACK:
        return {
          type: 'resource_limit',
          description: 'Add resource limits to prevent cascading failure',
          configuration: { memoryLimit: '512MB', timeoutMs: 60000 },
          effectiveness: this.calculateEffectiveness(pattern, strategy),
        };
      default:
        return null;
    }
  }

  async getRecommendedPrevention(
    agentId: string,
    phase: number
  ): Promise<PreventionStrategy[]> {
    const applicablePatterns: FailurePattern[] = [];

    for (const pattern of this.patterns.values()) {
      const [, , patternAgentType, patternPhase] = pattern.signature.split('::');

      if (patternAgentType === agentId.split('-')[0] || parseInt(patternPhase) === phase) {
        applicablePatterns.push(pattern);
      }
    }

    // Aggregate and sort by effectiveness
    const preventions = applicablePatterns
      .flatMap(p => p.preventionStrategies)
      .sort((a, b) => b.effectiveness - a.effectiveness);

    return preventions.slice(0, 5); // Top 5 preventions
  }
}
```

## Recovery Process

```typescript
export class RecoveryAgent {
  private diagnoser: FailureDiagnoser;
  private strategyEngine: RecoveryStrategyEngine;
  private executor: RecoveryExecutor;
  private learner: FailurePatternLearner;

  async handleFailure(failure: FailureContext): Promise<RecoveryReport> {
    console.log(`[Recovery Agent] Handling failure for ${failure.agentId} in phase ${failure.phase}`);

    // Step 1: Diagnose the failure
    const diagnosis = await this.diagnoser.diagnose(failure);
    console.log(`[Recovery Agent] Root cause: ${diagnosis.rootCause.category}`);

    // Step 2: Get prevention recommendations from learned patterns
    const preventions = await this.learner.getRecommendedPrevention(
      failure.agentId,
      failure.phase
    );

    // Step 3: Apply preventions to recovery options
    const options = diagnosis.recoveryOptions.map(option =>
      this.enhanceWithPreventions(option, preventions)
    );

    // Step 4: Execute recovery
    let execution: RecoveryExecution | null = null;
    let attempts = 0;

    while (attempts < 3 && (!execution || execution.finalState !== 'recovered')) {
      const option = options[attempts] || diagnosis.recommendedAction;
      console.log(`[Recovery Agent] Attempting recovery: ${option.strategy} (attempt ${attempts + 1})`);

      execution = await this.executor.executeRecovery(diagnosis, option);
      attempts++;
    }

    // Step 5: Learn from this recovery attempt
    if (execution) {
      await this.learner.learnFromRecovery(diagnosis, execution);
    }

    // Step 6: Generate report
    return this.generateRecoveryReport(diagnosis, execution, attempts);
  }

  private enhanceWithPreventions(
    option: RecoveryOption,
    preventions: PreventionStrategy[]
  ): RecoveryOption {
    const enhanced = { ...option };

    for (const prevention of preventions) {
      if (prevention.type === 'quality_hint') {
        // Add hint injection step
        enhanced.steps.unshift({
          order: 0,
          action: 'apply_prevention',
          memoryKey: 'coding/preventions/active',
          timeout: 5000,
        });
      }
    }

    return enhanced;
  }

  private generateRecoveryReport(
    diagnosis: DiagnosisResult,
    execution: RecoveryExecution | null,
    attempts: number
  ): RecoveryReport {
    return {
      failureId: diagnosis.failure.failureId,
      phase: diagnosis.failure.phase,
      agentId: diagnosis.failure.agentId,
      rootCause: diagnosis.rootCause,
      recoveryStatus: execution?.finalState || 'failed',
      attemptsRequired: attempts,
      recoveryStrategy: execution?.recoveryOption.strategy,
      duration: execution
        ? new Date(execution.endTime!).getTime() - new Date(execution.startTime).getTime()
        : 0,
      lessonsLearned: this.extractLessons(diagnosis, execution),
      preventionRecommendations: execution?.finalState === 'recovered'
        ? this.generatePreventionRecommendations(diagnosis)
        : [],
      timestamp: new Date().toISOString(),
    };
  }

  private extractLessons(
    diagnosis: DiagnosisResult,
    execution: RecoveryExecution | null
  ): string[] {
    const lessons: string[] = [];

    if (execution?.finalState === 'recovered') {
      lessons.push(`Recovery strategy '${execution.recoveryOption.strategy}' effective for ${diagnosis.rootCause.category}`);

      const successfulSteps = execution.stepResults.filter(r => r.status === 'success');
      if (successfulSteps.length > 0) {
        lessons.push(`Key recovery steps: ${successfulSteps.map(s => s.step.action).join(', ')}`);
      }
    } else {
      lessons.push(`All recovery attempts failed for ${diagnosis.rootCause.category}`);
      lessons.push('Manual intervention required');

      if (diagnosis.impactAssessment.dataLossRisk === 'high') {
        lessons.push('Consider adding more frequent checkpoints');
      }
    }

    return lessons;
  }
}
```

## Integration with Pipeline

### Failure Detection Hook

```typescript
// Hook that triggers recovery agent on failure detection
export const failureDetectionHook = {
  event: 'agent_failure',
  handler: async (context: HookContext) => {
    const failure: FailureContext = {
      failureId: generateId(),
      type: classifyFailure(context.error),
      phase: context.phase,
      agentId: context.agentId,
      timestamp: new Date().toISOString(),
      error: context.error,
      stackTrace: context.error.stack,
      memoryState: await retrieveMemoryState(),
      inputData: context.input,
      partialOutput: context.partialOutput,
    };

    const recoveryAgent = new RecoveryAgent();
    const report = await recoveryAgent.handleFailure(failure);

    // Store recovery report
    await storeMemory('coding/sherlock/recovery-report', report);

    // Notify if manual intervention needed
    if (report.recoveryStatus === 'failed') {
      await notify('pipeline-alerts', {
        level: 'critical',
        message: `Recovery failed for ${failure.agentId}`,
        report,
      });
    }

    return report.recoveryStatus === 'recovered';
  },
};
```

## MANDATORY: Feedback Verification Gate

**CRITICAL**: Before signing off on pipeline completion, recovery-agent MUST verify feedback was submitted.

### Feedback Verification Process

```typescript
export class FeedbackVerificationGate {
  async verifyFeedbackSubmitted(trajectoryId: string): Promise<FeedbackVerificationResult> {
    // Step 1: Check memory for feedback status
    const feedbackStatus = await this.retrieveMemory('coding/pipeline/feedback-status');

    if (!feedbackStatus) {
      return {
        verified: false,
        error: 'FEEDBACK_STATUS_MISSING',
        message: 'coding/pipeline/feedback-status not found - feedback was NOT submitted',
        action: 'HALT_PIPELINE',
      };
    }

    // Step 2: Verify trajectoryId matches
    if (feedbackStatus.trajectoryId !== trajectoryId) {
      return {
        verified: false,
        error: 'TRAJECTORY_MISMATCH',
        message: `Feedback trajectoryId (${feedbackStatus.trajectoryId}) does not match pipeline (${trajectoryId})`,
        action: 'HALT_PIPELINE',
      };
    }

    // Step 3: Verify feedback was actually submitted
    if (!feedbackStatus.feedbackSubmitted || !feedbackStatus.verified) {
      return {
        verified: false,
        error: 'FEEDBACK_NOT_VERIFIED',
        message: 'Feedback status exists but feedbackSubmitted or verified is false',
        action: 'RESUBMIT_FEEDBACK',
      };
    }

    // Step 4: Verify quality score exists
    if (typeof feedbackStatus.quality !== 'number' || feedbackStatus.quality < 0 || feedbackStatus.quality > 1) {
      return {
        verified: false,
        error: 'INVALID_QUALITY_SCORE',
        message: 'Quality score missing or invalid (must be 0.0-1.0)',
        action: 'RESUBMIT_FEEDBACK',
      };
    }

    // Step 5: Cross-verify with learning.db (optional but recommended)
    const dbVerification = await this.verifyInLearningDatabase(trajectoryId);

    return {
      verified: true,
      quality: feedbackStatus.quality,
      timestamp: feedbackStatus.timestamp,
      dbVerified: dbVerification,
    };
  }

  async enforceGate(trajectoryId: string): Promise<void> {
    const result = await this.verifyFeedbackSubmitted(trajectoryId);

    if (!result.verified) {
      // Store failure reason
      await this.storeMemory('coding/forensic/feedback-gate-failure', {
        trajectoryId,
        error: result.error,
        message: result.message,
        action: result.action,
        timestamp: new Date().toISOString(),
      });

      // HALT PIPELINE
      throw new FeedbackGateError(
        `FEEDBACK GATE FAILED: ${result.error} - ${result.message}. ` +
        `Action required: ${result.action}. ` +
        `Pipeline CANNOT complete without verified feedback submission.`
      );
    }

    console.log(`[recovery-agent] Feedback gate PASSED: quality=${result.quality}, verified at ${result.timestamp}`);
  }
}
```

### Feedback Gate in Recovery Flow

```typescript
export class RecoveryAgent {
  private feedbackGate: FeedbackVerificationGate;

  async handlePipelineCompletion(context: PipelineContext): Promise<RecoveryReport> {
    // ... existing failure handling code ...

    // MANDATORY: Verify feedback before allowing completion
    console.log('[recovery-agent] Enforcing MANDATORY feedback gate...');
    await this.feedbackGate.enforceGate(context.trajectoryId);

    // Only reach here if feedback gate passed
    return this.generateFinalReport(context, { feedbackVerified: true });
  }
}
```

### Memory Keys for Feedback Verification

| Key | Purpose | Required |
|-----|---------|----------|
| `coding/pipeline/feedback-status` | Stores feedback submission confirmation | **MANDATORY** |
| `coding/forensic/feedback-gate-failure` | Stores gate failure details if failed | On failure |
| `coding/forensic/final-report` | Final recovery report with feedbackVerified flag | **MANDATORY** |

## Quality Checklist

### Failure Diagnosis
- [ ] Root cause correctly identified
- [ ] All affected agents determined
- [ ] Impact accurately assessed
- [ ] Evidence collected and preserved

### Recovery Execution
- [ ] Appropriate strategy selected
- [ ] All steps executed in order
- [ ] Rollback capability maintained
- [ ] Recovery validated after completion

### Learning & Prevention
- [ ] Failure pattern recorded
- [ ] Recovery effectiveness tracked
- [ ] Prevention strategies derived
- [ ] Patterns persisted for future use

### **MANDATORY: Feedback Verification (NEW)**
- [ ] `coding/pipeline/feedback-status` exists
- [ ] `feedbackSubmitted` is `true`
- [ ] `verified` is `true`
- [ ] `quality` is valid (0.0-1.0)
- [ ] `trajectoryId` matches pipeline
- [ ] Cross-verified with learning.db (recommended)

### Escalation
- [ ] Failed recoveries escalated promptly
- [ ] Clear escalation path documented
- [ ] Manual intervention guidance provided
- [ ] Stakeholders notified appropriately
