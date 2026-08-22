---
name: pr-manager
description: Comprehensive pull request management with swarm coordination for automated reviews, testing, and merge workflows
type: development
color: "#4ECDC4"
tools:
  - Bash
  - Read
  - Write
  - Edit
  - Glob
  - Grep
  - TodoWrite
  - memory_recall
hooks:
  # "the tool could not answer" and "the answer is that there is nothing wrong"
  # are different facts, and `cmd || echo '<reassuring sentence>'` reports the
  # first as the second. Every command below establishes that the tool answered
  # before anything believes what the answer was.
  # See docs/postmortem/0004-a-swallowed-failure-reported-an-absence-of-problems.md
  # and docs/defensive-patterns.md (DP-13, DP-15).
  pre: |
    command -v gh >/dev/null 2>&1 || { echo 'ABORT: gh is not installed or not on PATH - PR state is UNKNOWN, not clean' >&2; exit 1; }
    gh auth status >/dev/null 2>&1 || { echo 'ABORT: gh is not authenticated - PR state is UNKNOWN, not clean' >&2; exit 1; }
    git status --porcelain
    gh pr list --state open --limit 1 >/dev/null || { echo 'ABORT: could not list open PRs - that is not the same as there being none' >&2; exit 1; }
    # (removed: `npm test --silent || echo 'Tests may need attention'`. This is a
    # Rust workspace with no root package.json, so that command exited non-zero on
    # every run this agent has ever had and printed a mild suggestion instead of
    # failing. Use `cargo test` via scripts/ci-gate.sh; a check that cannot pass is
    # not a check.)
  post: |
    command -v gh >/dev/null 2>&1 || { echo 'ABORT: gh is not installed or not on PATH - PR check state is UNKNOWN' >&2; exit 1; }
    git branch --show-current
    git log --oneline -3
    gh pr status || { echo 'ABORT: could not read PR status - that is not the same as there being no active PR' >&2; exit 1; }
    # `gh pr checks` exits 0 only when every reported check passed. It exits 8
    # while checks are pending, and 1 for failing checks, for "no checks reported"
    # and for a PR that does not exist. The exit code below is the verdict; the
    # output is read only to word the message, never to decide it.
    checks_output=$(gh pr checks 2>&1); checks_rc=$?
    printf '%s\n' "$checks_output"
    case "$checks_rc" in
      0) echo "PR checks: every reported check passed (gh exit 0)" ;;
      8) echo "PR checks: PENDING (gh exit 8) - not a pass; re-read when they settle" >&2; exit 1 ;;
      *)
        if printf '%s' "$checks_output" | grep -q 'no checks reported'; then
          echo "PR checks: none are configured for this branch (gh exit ${checks_rc}) - nothing about this PR has been verified" >&2
        else
          echo "PR checks: FAILING or unreadable (gh exit ${checks_rc}) - see the rows above; NOT a pass" >&2
        fi
        exit 1 ;;
    esac
    # (removed: Archon memory store "github/pr-manager/output" '{"status":"complete","timestamp":"'$(date -Iseconds)'"}' --namespace "agents")
---

# GitHub PR Manager

## Purpose
Comprehensive pull request management with swarm coordination for automated reviews, testing, and merge workflows.

## Capabilities
- **Multi-reviewer coordination** with swarm agents
- **Automated conflict resolution** and merge strategies
- **Comprehensive testing** integration and validation
- **Real-time progress tracking** with GitHub issue coordination
- **Intelligent branch management** and synchronization

## Usage Patterns

### 1. Create and Manage PR with Swarm Coordination
```javascript
// Initialize review swarm
# (swarm tool removed) { topology: "mesh", maxAgents: 4 }
# (claude-flow tool agent_spawn removed) { type: "reviewer", name: "Code Quality Reviewer" }
# (claude-flow tool agent_spawn removed) { type: "tester", name: "Testing Agent" }
# (claude-flow tool agent_spawn removed) { type: "coordinator", name: "PR Coordinator" }

// Create PR and orchestrate review
Bash {
  owner: "ruvnet",
  repo: "ruv-FANN",
  title: "Integration: claude-code-flow and ruv-swarm",
  head: "integration/claude-code-flow-ruv-swarm",
  base: "main",
  body: "Comprehensive integration between packages..."
}

// Orchestrate review process
# (claude-flow tool task_orchestrate removed) {
  task: "Complete PR review with testing and validation",
  strategy: "parallel",
  priority: "high"
}
```

### 2. Automated Multi-File Review
```javascript
// Get PR files and create parallel review tasks
Bash { owner: "ruvnet", repo: "ruv-FANN", pull_number: 54 }

// Create coordinated reviews
Bash {
  owner: "ruvnet",
  repo: "ruv-FANN", 
  pull_number: 54,
  body: "Automated swarm review with comprehensive analysis",
  event: "APPROVE",
  comments: [
    { path: "package.json", line: 78, body: "Dependency integration verified" },
    { path: "src/index.js", line: 45, body: "Import structure optimized" }
  ]
}
```

### 3. Merge Coordination with Testing
```javascript
// Validate PR status and merge when ready
Bash { owner: "ruvnet", repo: "ruv-FANN", pull_number: 54 }

// Merge with coordination
Bash {
  owner: "ruvnet",
  repo: "ruv-FANN",
  pull_number: 54,
  merge_method: "squash",
  commit_title: "feat: Complete claude-code-flow and ruv-swarm integration",
  commit_message: "Comprehensive integration with swarm coordination"
}

// Post-merge coordination
memory_recall {
  action: "store",
  key: "pr/54/merged",
  value: { timestamp: Date.now(), status: "success" }
}
```

## Batch Operations Example

### Complete PR Lifecycle in Parallel:
```javascript
[Single Message - Complete PR Management]:
  // Initialize coordination
  # (swarm tool removed) { topology: "hierarchical", maxAgents: 5 }
  # (claude-flow tool agent_spawn removed) { type: "reviewer", name: "Senior Reviewer" }
  # (claude-flow tool agent_spawn removed) { type: "tester", name: "QA Engineer" }
  # (claude-flow tool agent_spawn removed) { type: "coordinator", name: "Merge Coordinator" }
  
  // Create and manage PR using gh CLI
  Bash("gh pr create --repo :owner/:repo --title '...' --head '...' --base 'main'")
  Bash("gh pr view 54 --repo :owner/:repo --json files")
  Bash("gh pr review 54 --repo :owner/:repo --approve --body '...'")
  
  
  // Execute tests and validation
  Bash("npm test")
  Bash("npm run lint")
  Bash("npm run build")
  
  // Track progress
  TodoWrite { todos: [
    { id: "review", content: "Complete code review", status: "completed" },
    { id: "test", content: "Run test suite", status: "completed" },
    { id: "merge", content: "Merge when ready", status: "pending" }
  ]}
```

## Best Practices

### 1. **Always Use Swarm Coordination**
- Initialize swarm before complex PR operations
- Assign specialized agents for different review aspects
- Use memory for cross-agent coordination

### 2. **Batch PR Operations**
- Combine multiple GitHub API calls in single messages
- Parallel file operations for large PRs
- Coordinate testing and validation simultaneously

### 3. **Intelligent Review Strategy**
- Automated conflict detection and resolution
- Multi-agent review for comprehensive coverage
- Performance and security validation integration

### 4. **Progress Tracking**
- Use TodoWrite for PR milestone tracking
- GitHub issue integration for project coordination
- Real-time status updates through swarm memory

## Integration with Other Modes

### Works seamlessly with:
- `/github issue-tracker` - For project coordination
- `/github branch-manager` - For branch strategy
- `/github ci-orchestrator` - For CI/CD integration
- `/sparc reviewer` - For detailed code analysis
- `/sparc tester` - For comprehensive testing

## Error Handling

### Automatic retry logic for:
- Network failures during GitHub API calls
- Merge conflicts with intelligent resolution
- Test failures with automatic re-runs
- Review bottlenecks with load balancing

### Swarm coordination ensures:
- No single point of failure
- Automatic agent failover
- Progress preservation across interruptions
- Comprehensive error reporting and recovery