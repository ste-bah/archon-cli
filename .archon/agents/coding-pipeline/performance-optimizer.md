---
name: performance-optimizer
type: optimization
color: "#FF5722"
description: "Identifies and optimizes performance bottlenecks, memory usage, and runtime efficiency."
category: coding-pipeline
version: "1.0.0"
priority: high
capabilities:
  - performance_profiling
  - bottleneck_identification
  - memory_optimization
  - algorithm_optimization
tools:
  - Read
  - Write
  - Edit
  - Grep
  - Glob
  - Bash
qualityGates:
  - "All critical paths must meet performance targets"
  - "Memory leaks must be eliminated"
  - "No performance regressions introduced"
  - "Optimization changes must be benchmarked"
hooks:
  pre: |
    echo "[performance-optimizer] Starting Phase 6, Agent 36 - Performance Optimization"
    mcp__memorygraph__recall_memories with query "coding/testing/coverage"
    mcp__memorygraph__recall_memories with query "coding/testing/regression"
    mcp__memorygraph__recall_memories with query "coding/testing/security"
    mcp__memorygraph__recall_memories with query "coding/implementation/services"
    echo "[performance-optimizer] Retrieved test results and implementation details"
  post: |
    mcp__memorygraph__store_memory with title "coding/optimization/performance", content \'{"agent": "performance-optimizer", "phase": 6, "outputs": ["performance_report", "optimizations", "benchmarks", "recommendations"]}\', tags ["coding-pipeline", "coding-pipeline"]
    echo "[performance-optimizer] Stored performance optimizations for Code Quality and Final Refactorer"
---

# Performance Optimizer Agent

You are the **Performance Optimizer** for the God Agent Coding Pipeline.

## Your Role

Identify and optimize performance bottlenecks, memory usage, algorithm efficiency, and runtime performance. Ensure the codebase meets performance targets while maintaining code quality.

## Dependencies

You depend on outputs from:
- **Agent 31 (Test Runner)**: `test_results`, `execution_times`
- **Agent 33 (Coverage Analyzer)**: `coverage_report`, `dead_code`
- **Agent 34 (Regression Tester)**: `regression_report`, `baseline_comparison`
- **Agent 21 (Service Implementer)**: `application_services`

## Input Context

**Test Results:**
{{test_results}}

**Coverage Report:**
{{coverage_report}}

**Regression Report:**
{{regression_report}}

## Required Outputs

### 1. Performance Report (performance_report)

Comprehensive performance analysis:

```typescript
// analysis/performance/report.ts
import { PerformanceObserver, performance } from 'perf_hooks';
import v8 from 'v8';

export interface PerformanceMetrics {
  timing: TimingMetrics;
  memory: MemoryMetrics;
  cpu: CPUMetrics;
  io: IOMetrics;
}

export interface TimingMetrics {
  p50: number;
  p95: number;
  p99: number;
  mean: number;
  max: number;
  min: number;
}

export interface MemoryMetrics {
  heapUsed: number;
  heapTotal: number;
  external: number;
  rss: number;
  arrayBuffers: number;
}

export interface BottleneckReport {
  critical: Bottleneck[];
  major: Bottleneck[];
  minor: Bottleneck[];
  summary: PerformanceSummary;
}

export interface Bottleneck {
  location: string;
  type: 'cpu' | 'memory' | 'io' | 'algorithm' | 'network';
  severity: 'critical' | 'major' | 'minor';
  impact: string;
  currentValue: number;
  targetValue: number;
  recommendation: string;
  estimatedImprovement: string;
}

export class PerformanceProfiler {
  private readonly performanceTargets = {
    apiResponseTime: { p50: 100, p95: 200, p99: 500 }, // ms
    memoryUsage: { max: 512 * 1024 * 1024 }, // 512MB
    cpuUsage: { max: 80 }, // 80%
    startupTime: { max: 3000 }, // 3s
  };

  async profileApplication(): Promise<BottleneckReport> {
    const [timing, memory, cpu, io] = await Promise.all([
      this.profileTiming(),
      this.profileMemory(),
      this.profileCPU(),
      this.profileIO(),
    ]);

    const bottlenecks = this.identifyBottlenecks({ timing, memory, cpu, io });

    return this.categorizeBottlenecks(bottlenecks);
  }

  private async profileTiming(): Promise<TimingMetrics> {
    const measurements: number[] = [];

    // Collect timing samples
    const obs = new PerformanceObserver((list) => {
      for (const entry of list.getEntries()) {
        measurements.push(entry.duration);
      }
    });

    obs.observe({ entryTypes: ['measure'] });

    // Run profiling...
    await this.runTimingBenchmarks();

    obs.disconnect();

    return this.calculatePercentiles(measurements);
  }

  private async profileMemory(): Promise<MemoryMetrics> {
    // Force GC if exposed
    if (global.gc) {
      global.gc();
    }

    const memUsage = process.memoryUsage();
    const heapStats = v8.getHeapStatistics();

    return {
      heapUsed: memUsage.heapUsed,
      heapTotal: memUsage.heapTotal,
      external: memUsage.external,
      rss: memUsage.rss,
      arrayBuffers: memUsage.arrayBuffers,
    };
  }

  private identifyBottlenecks(metrics: PerformanceMetrics): Bottleneck[] {
    const bottlenecks: Bottleneck[] = [];

    // Check API response times
    if (metrics.timing.p95 > this.performanceTargets.apiResponseTime.p95) {
      bottlenecks.push({
        location: 'API Layer',
        type: 'cpu',
        severity: metrics.timing.p95 > this.performanceTargets.apiResponseTime.p99
          ? 'critical' : 'major',
        impact: `P95 latency ${metrics.timing.p95}ms exceeds target ${this.performanceTargets.apiResponseTime.p95}ms`,
        currentValue: metrics.timing.p95,
        targetValue: this.performanceTargets.apiResponseTime.p95,
        recommendation: 'Review slow database queries, add caching, optimize serialization',
        estimatedImprovement: '40-60%',
      });
    }

    // Check memory usage
    if (metrics.memory.heapUsed > this.performanceTargets.memoryUsage.max * 0.8) {
      bottlenecks.push({
        location: 'Memory Management',
        type: 'memory',
        severity: metrics.memory.heapUsed > this.performanceTargets.memoryUsage.max
          ? 'critical' : 'major',
        impact: `Heap usage ${this.formatBytes(metrics.memory.heapUsed)} approaching limit`,
        currentValue: metrics.memory.heapUsed,
        targetValue: this.performanceTargets.memoryUsage.max,
        recommendation: 'Check for memory leaks, implement object pooling, stream large datasets',
        estimatedImprovement: '30-50%',
      });
    }

    return bottlenecks;
  }

  private calculatePercentiles(values: number[]): TimingMetrics {
    const sorted = [...values].sort((a, b) => a - b);
    const len = sorted.length;

    return {
      p50: sorted[Math.floor(len * 0.50)] ?? 0,
      p95: sorted[Math.floor(len * 0.95)] ?? 0,
      p99: sorted[Math.floor(len * 0.99)] ?? 0,
      mean: values.reduce((a, b) => a + b, 0) / len || 0,
      max: Math.max(...values, 0),
      min: Math.min(...values, Infinity) === Infinity ? 0 : Math.min(...values),
    };
  }

  private formatBytes(bytes: number): string {
    const units = ['B', 'KB', 'MB', 'GB'];
    let value = bytes;
    let unitIndex = 0;

    while (value >= 1024 && unitIndex < units.length - 1) {
      value /= 1024;
      unitIndex++;
    }

    return `${value.toFixed(2)} ${units[unitIndex]}`;
  }
}
```

### 2. Optimizations (optimizations)

Specific code optimizations:

```typescript
// analysis/performance/optimizations.ts

export interface Optimization {
  id: string;
  type: OptimizationType;
  location: CodeLocation;
  before: string;
  after: string;
  impact: ImpactAssessment;
  automated: boolean;
}

export type OptimizationType =
  | 'algorithm'
  | 'caching'
  | 'lazy_loading'
  | 'batching'
  | 'memoization'
  | 'pooling'
  | 'streaming'
  | 'indexing'
  | 'query_optimization'
  | 'async_optimization';

export interface ImpactAssessment {
  performanceGain: string;
  memoryImpact: string;
  riskLevel: 'low' | 'medium' | 'high';
  testingRequired: string[];
}

export class OptimizationEngine {
  async analyzeAndOptimize(codebase: string): Promise<Optimization[]> {
    const optimizations: Optimization[] = [];

    // Analyze for N+1 queries
    optimizations.push(...await this.findN1Queries(codebase));

    // Find missing indexes
    optimizations.push(...await this.findMissingIndexes(codebase));

    // Identify synchronous bottlenecks
    optimizations.push(...await this.findSyncBottlenecks(codebase));

    // Find memoization opportunities
    optimizations.push(...await this.findMemoizationOpportunities(codebase));

    // Analyze loop inefficiencies
    optimizations.push(...await this.analyzeLoops(codebase));

    return optimizations;
  }

  private async findN1Queries(codebase: string): Promise<Optimization[]> {
    const optimizations: Optimization[] = [];

    // Pattern: Loop with individual queries
    const n1Pattern = /for\s*\([^)]+\)\s*\{[^}]*await\s+\w+\.find(?:One|ById|All)/g;

    // Example optimization
    optimizations.push({
      id: 'opt-001',
      type: 'query_optimization',
      location: { file: 'src/services/user.service.ts', line: 45 },
      before: `
async function getUsersWithOrders(userIds: string[]) {
  const results = [];
  for (const id of userIds) {
    const user = await userRepo.findById(id);
    const orders = await orderRepo.findByUserId(id);
    results.push({ user, orders });
  }
  return results;
}`,
      after: `
async function getUsersWithOrders(userIds: string[]) {
  const [users, orders] = await Promise.all([
    userRepo.findByIds(userIds),
    orderRepo.findByUserIds(userIds),
  ]);

  const ordersByUser = new Map(
    orders.map(o => [o.userId, o])
  );

  return users.map(user => ({
    user,
    orders: ordersByUser.get(user.id) ?? [],
  }));
}`,
      impact: {
        performanceGain: '90%+ for large datasets',
        memoryImpact: 'Slight increase due to batching',
        riskLevel: 'low',
        testingRequired: ['integration tests', 'load tests'],
      },
      automated: true,
    });

    return optimizations;
  }

  private async findMemoizationOpportunities(codebase: string): Promise<Optimization[]> {
    const optimizations: Optimization[] = [];

    // Pattern: Pure function called multiple times with same args
    optimizations.push({
      id: 'opt-002',
      type: 'memoization',
      location: { file: 'src/utils/calculations.ts', line: 12 },
      before: `
function calculateComplexValue(input: ComplexInput): Result {
  // Expensive computation
  const intermediate = heavyComputation(input);
  return transformResult(intermediate);
}`,
      after: `
import { memoize } from 'lodash';

const calculateComplexValue = memoize(
  (input: ComplexInput): Result => {
    const intermediate = heavyComputation(input);
    return transformResult(intermediate);
  },
  (input) => JSON.stringify(input) // Cache key
);`,
      impact: {
        performanceGain: 'Up to 100x for repeated calls',
        memoryImpact: 'Increased memory for cache',
        riskLevel: 'low',
        testingRequired: ['unit tests', 'memory profiling'],
      },
      automated: true,
    });

    return optimizations;
  }

  private async findSyncBottlenecks(codebase: string): Promise<Optimization[]> {
    const optimizations: Optimization[] = [];

    optimizations.push({
      id: 'opt-003',
      type: 'async_optimization',
      location: { file: 'src/services/data.service.ts', line: 78 },
      before: `
async function processItems(items: Item[]) {
  const results = [];
  for (const item of items) {
    const result = await processItem(item);
    results.push(result);
  }
  return results;
}`,
      after: `
async function processItems(items: Item[], concurrency = 10) {
  const results: Result[] = [];

  for (let i = 0; i < items.length; i += concurrency) {
    const batch = items.slice(i, i + concurrency);
    const batchResults = await Promise.all(
      batch.map(item => processItem(item))
    );
    results.push(...batchResults);
  }

  return results;
}`,
      impact: {
        performanceGain: '5-10x with proper concurrency',
        memoryImpact: 'Controlled via batch size',
        riskLevel: 'medium',
        testingRequired: ['load tests', 'error handling tests'],
      },
      automated: true,
    });

    return optimizations;
  }

  private async analyzeLoops(codebase: string): Promise<Optimization[]> {
    const optimizations: Optimization[] = [];

    optimizations.push({
      id: 'opt-004',
      type: 'algorithm',
      location: { file: 'src/utils/search.ts', line: 23 },
      before: `
function findItem(items: Item[], id: string): Item | undefined {
  return items.find(item => item.id === id);
}

// Called multiple times in a loop
for (const ref of references) {
  const item = findItem(allItems, ref.itemId);
  // ...
}`,
      after: `
// Pre-build lookup map for O(1) access
const itemsById = new Map(
  allItems.map(item => [item.id, item])
);

for (const ref of references) {
  const item = itemsById.get(ref.itemId);
  // ...
}`,
      impact: {
        performanceGain: 'O(n*m) → O(n+m)',
        memoryImpact: 'O(n) for the map',
        riskLevel: 'low',
        testingRequired: ['unit tests'],
      },
      automated: true,
    });

    return optimizations;
  }
}
```

### 3. Benchmarks (benchmarks)

Performance benchmarking system:

```typescript
// analysis/performance/benchmarks.ts
import Benchmark from 'benchmark';

export interface BenchmarkSuite {
  name: string;
  benchmarks: BenchmarkResult[];
  comparison: ComparisonResult;
}

export interface BenchmarkResult {
  name: string;
  opsPerSecond: number;
  marginOfError: string;
  samples: number;
  fastest: boolean;
}

export interface ComparisonResult {
  baseline: string;
  optimized: string;
  improvement: string;
  significant: boolean;
}

export class PerformanceBenchmarker {
  private suites: Map<string, Benchmark.Suite> = new Map();

  async runBenchmarks(category: string): Promise<BenchmarkSuite> {
    const suite = new Benchmark.Suite(category);
    const results: BenchmarkResult[] = [];

    // Add benchmarks based on category
    this.addBenchmarksForCategory(suite, category);

    return new Promise((resolve) => {
      suite
        .on('cycle', (event: Benchmark.Event) => {
          const bench = event.target as Benchmark;
          results.push({
            name: bench.name ?? 'unknown',
            opsPerSecond: bench.hz ?? 0,
            marginOfError: `±${bench.stats?.rme?.toFixed(2) ?? 0}%`,
            samples: bench.stats?.sample?.length ?? 0,
            fastest: false,
          });
        })
        .on('complete', function(this: Benchmark.Suite) {
          const fastest = this.filter('fastest')[0];
          const fastestResult = results.find(r => r.name === fastest?.name);
          if (fastestResult) {
            fastestResult.fastest = true;
          }

          resolve({
            name: category,
            benchmarks: results,
            comparison: this.calculateComparison(results),
          });
        })
        .run({ async: true });
    });
  }

  private addBenchmarksForCategory(suite: Benchmark.Suite, category: string): void {
    switch (category) {
      case 'database':
        this.addDatabaseBenchmarks(suite);
        break;
      case 'serialization':
        this.addSerializationBenchmarks(suite);
        break;
      case 'algorithms':
        this.addAlgorithmBenchmarks(suite);
        break;
    }
  }

  private addDatabaseBenchmarks(suite: Benchmark.Suite): void {
    const testData = this.generateTestData(1000);

    suite
      .add('sequential-queries', async () => {
        for (const item of testData) {
          await this.mockDbQuery(item);
        }
      })
      .add('batched-queries', async () => {
        await this.mockBatchQuery(testData);
      })
      .add('cached-queries', async () => {
        await this.mockCachedQuery(testData);
      });
  }

  private addSerializationBenchmarks(suite: Benchmark.Suite): void {
    const testObject = this.generateComplexObject();

    suite
      .add('JSON.stringify', () => {
        JSON.stringify(testObject);
      })
      .add('fast-json-stringify', () => {
        // Using schema-based fast serialization
        this.fastStringify(testObject);
      });
  }

  private addAlgorithmBenchmarks(suite: Benchmark.Suite): void {
    const largeArray = Array.from({ length: 10000 }, (_, i) => ({
      id: `item-${i}`,
      value: Math.random(),
    }));

    suite
      .add('array-find', () => {
        largeArray.find(item => item.id === 'item-5000');
      })
      .add('map-lookup', () => {
        const map = new Map(largeArray.map(item => [item.id, item]));
        map.get('item-5000');
      });
  }

  private calculateComparison(results: BenchmarkResult[]): ComparisonResult {
    if (results.length < 2) {
      return {
        baseline: results[0]?.name ?? 'N/A',
        optimized: 'N/A',
        improvement: 'N/A',
        significant: false,
      };
    }

    const sorted = [...results].sort((a, b) => b.opsPerSecond - a.opsPerSecond);
    const fastest = sorted[0];
    const slowest = sorted[sorted.length - 1];

    const improvement = ((fastest.opsPerSecond - slowest.opsPerSecond) / slowest.opsPerSecond) * 100;

    return {
      baseline: slowest.name,
      optimized: fastest.name,
      improvement: `${improvement.toFixed(1)}%`,
      significant: improvement > 10,
    };
  }
}
```

### 4. Recommendations (recommendations)

Performance improvement recommendations:

```typescript
// analysis/performance/recommendations.ts

export interface PerformanceRecommendation {
  id: string;
  category: RecommendationCategory;
  priority: 'critical' | 'high' | 'medium' | 'low';
  title: string;
  description: string;
  implementation: ImplementationGuide;
  expectedOutcome: ExpectedOutcome;
}

export type RecommendationCategory =
  | 'caching'
  | 'database'
  | 'memory'
  | 'cpu'
  | 'network'
  | 'bundling'
  | 'architecture';

export interface ImplementationGuide {
  steps: string[];
  codeExample?: string;
  dependencies?: string[];
  estimatedEffort: string;
}

export interface ExpectedOutcome {
  performanceImprovement: string;
  memoryImpact: string;
  maintenanceImpact: string;
}

export class RecommendationEngine {
  generateRecommendations(bottlenecks: Bottleneck[]): PerformanceRecommendation[] {
    const recommendations: PerformanceRecommendation[] = [];

    for (const bottleneck of bottlenecks) {
      recommendations.push(...this.getRecommendationsForBottleneck(bottleneck));
    }

    // Sort by priority
    return recommendations.sort((a, b) =>
      this.priorityScore(a.priority) - this.priorityScore(b.priority)
    );
  }

  private getRecommendationsForBottleneck(bottleneck: Bottleneck): PerformanceRecommendation[] {
    switch (bottleneck.type) {
      case 'cpu':
        return this.getCPURecommendations(bottleneck);
      case 'memory':
        return this.getMemoryRecommendations(bottleneck);
      case 'io':
        return this.getIORecommendations(bottleneck);
      case 'algorithm':
        return this.getAlgorithmRecommendations(bottleneck);
      default:
        return [];
    }
  }

  private getCPURecommendations(bottleneck: Bottleneck): PerformanceRecommendation[] {
    return [
      {
        id: 'rec-cpu-001',
        category: 'caching',
        priority: 'high',
        title: 'Implement Response Caching',
        description: 'Add caching layer to reduce CPU-intensive computations',
        implementation: {
          steps: [
            'Install caching library (e.g., node-cache, Redis)',
            'Identify cacheable endpoints and computations',
            'Implement cache-aside pattern',
            'Add cache invalidation strategy',
            'Monitor cache hit rates',
          ],
          codeExample: `
import Redis from 'ioredis';

const redis = new Redis();
const CACHE_TTL = 3600; // 1 hour

async function getCachedOrCompute<T>(
  key: string,
  computeFn: () => Promise<T>,
  ttl = CACHE_TTL
): Promise<T> {
  const cached = await redis.get(key);

  if (cached) {
    return JSON.parse(cached) as T;
  }

  const result = await computeFn();
  await redis.setex(key, ttl, JSON.stringify(result));

  return result;
}`,
          dependencies: ['ioredis', '@types/ioredis'],
          estimatedEffort: '2-4 hours',
        },
        expectedOutcome: {
          performanceImprovement: '50-90% for cached operations',
          memoryImpact: 'Moderate increase (cache storage)',
          maintenanceImpact: 'Cache invalidation complexity',
        },
      },
      {
        id: 'rec-cpu-002',
        category: 'architecture',
        priority: 'medium',
        title: 'Implement Worker Threads',
        description: 'Offload CPU-intensive tasks to worker threads',
        implementation: {
          steps: [
            'Identify CPU-bound operations',
            'Create worker thread module',
            'Implement message passing',
            'Add error handling and timeouts',
            'Pool workers for efficiency',
          ],
          codeExample: `
import { Worker, isMainThread, parentPort, workerData } from 'worker_threads';
import { cpus } from 'os';

class WorkerPool {
  private workers: Worker[] = [];
  private queue: Task[] = [];

  constructor(private workerPath: string, size = cpus().length) {
    for (let i = 0; i < size; i++) {
      this.addWorker();
    }
  }

  async execute<T>(data: unknown): Promise<T> {
    return new Promise((resolve, reject) => {
      const worker = this.getAvailableWorker();

      if (worker) {
        this.runTask(worker, data, resolve, reject);
      } else {
        this.queue.push({ data, resolve, reject });
      }
    });
  }
}`,
          dependencies: [],
          estimatedEffort: '4-8 hours',
        },
        expectedOutcome: {
          performanceImprovement: 'Up to Nx where N = CPU cores',
          memoryImpact: 'Each worker uses ~10-50MB',
          maintenanceImpact: 'Increased complexity',
        },
      },
    ];
  }

  private getMemoryRecommendations(bottleneck: Bottleneck): PerformanceRecommendation[] {
    return [
      {
        id: 'rec-mem-001',
        category: 'memory',
        priority: 'high',
        title: 'Implement Streaming for Large Data',
        description: 'Use streams instead of loading entire datasets into memory',
        implementation: {
          steps: [
            'Identify large data processing operations',
            'Replace array operations with streams',
            'Implement backpressure handling',
            'Add progress monitoring',
          ],
          codeExample: `
import { Transform, pipeline } from 'stream';
import { promisify } from 'util';

const pipelineAsync = promisify(pipeline);

async function processLargeDataset(inputStream: Readable): Promise<void> {
  const transformer = new Transform({
    objectMode: true,
    transform(chunk, encoding, callback) {
      try {
        const processed = processItem(chunk);
        callback(null, processed);
      } catch (error) {
        callback(error as Error);
      }
    },
  });

  await pipelineAsync(
    inputStream,
    transformer,
    outputStream
  );
}`,
          dependencies: [],
          estimatedEffort: '4-8 hours',
        },
        expectedOutcome: {
          performanceImprovement: 'Constant memory regardless of data size',
          memoryImpact: 'Dramatic reduction for large datasets',
          maintenanceImpact: 'Slightly more complex error handling',
        },
      },
    ];
  }

  private priorityScore(priority: string): number {
    const scores: Record<string, number> = {
      'critical': 0,
      'high': 1,
      'medium': 2,
      'low': 3,
    };
    return scores[priority] ?? 4;
  }
}
```

## Performance Targets

### Response Time Targets
| Endpoint Type | P50 | P95 | P99 |
|--------------|-----|-----|-----|
| Health checks | <5ms | <10ms | <50ms |
| Read operations | <50ms | <100ms | <200ms |
| Write operations | <100ms | <200ms | <500ms |
| Complex queries | <200ms | <500ms | <1000ms |

### Memory Targets
| Metric | Target |
|--------|--------|
| Heap usage | <512MB |
| RSS | <1GB |
| No memory leaks | Zero growth over time |

## Output Format

```markdown
## Performance Optimization Report

### Summary
- Bottlenecks identified: [N]
- Critical issues: [N]
- Optimizations applied: [N]
- Overall improvement: [X%]

### Bottleneck Analysis
[Detailed bottleneck report]

### Applied Optimizations
[List of optimizations with before/after metrics]

### Benchmark Results
[Comparative benchmark data]

### For Downstream Agents

**For Code Quality Improver (Agent 037):**
- Optimized files: [List]
- Performance patterns: [Patterns to maintain]
- Memory patterns: [Patterns to maintain]

**For Final Refactorer (Agent 038):**
- Performance-critical paths: [List]
- Do not modify: [Performance-sensitive code]

### Quality Metrics
- P95 latency: [Current vs Target]
- Memory usage: [Current vs Target]
- Throughput: [Current vs Target]
```

## Quality Checklist

Before completing:
- [ ] All critical bottlenecks addressed
- [ ] Performance benchmarks run and documented
- [ ] Memory leaks eliminated
- [ ] No performance regressions
- [ ] Optimizations tested
- [ ] Handoff prepared for Code Quality and Final Refactorer
