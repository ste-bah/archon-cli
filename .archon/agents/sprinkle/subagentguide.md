---
name: subagent-guide
description: Comprehensive guide to Claude Code subagents covering architecture, orchestration patterns, MCP integration, hooks, and best practices for multi-agent workflows.
---

I'll do comprehensive research on Claude Code subagents for you. Let me search across multiple sources to build a complete picture.Based on my deep research, here's your comprehensive guide to Claude Code subagents:

# **The Complete Claude Code Subagents Deep Dive**

## **What You Already Know (Validated)**

Your uploaded document is excellent and accurate. Everything in it aligns with current best practices. Let me now expand massively on what's NOT in there.

---

## **1. FUNDAMENTAL ARCHITECTURE PRINCIPLES**

### **The Core Paradigm Shift**
Subagents represent moving from **"one AI assistant"** to **"a specialized AI team"**. This mirrors software architecture evolution:
- Monolithic → Microservices
- Single-threaded → Parallel processing
- Generalist → Specialist

### **Critical Technical Details**

**Context Windows:**
- Each subagent gets its own **independent 200K token context**
- Main agent preserves its context when delegating
- Multi-agent sessions consume **3-4x more tokens** than single-threaded
- Token optimization is critical for cost management

**Performance Benchmarks:**
- Claude Code with subagents achieved **72.5% on SWE-bench** (July 2025)
- Well-orchestrated teams complete **45-minute features in under 10 minutes**
- Up to **10 concurrent subagents** can run simultaneously
- Queuing systems successfully manage **100+ agents** through batch processing

**Execution Model:**
- Subagents execute **sequentially within a single REPL session**
- For true parallelism, open multiple Claude Code sessions
- Subagents **CANNOT spawn other subagents** (prevents infinite nesting)
- Built-in subagents: **Plan subagent** (plan mode) and **Explore subagent** (Haiku-powered search)

---

## **2. ADVANCED FILE STRUCTURE & PRIORITY**

### **Location Hierarchy (Precedence Order)**

```
1. CLI-defined (--agents flag) [Highest for dynamic use]
2. Project-level (.claude/agents/) [Highest for version control]
3. User-level (~/.claude/agents/) [Lowest - global defaults]
4. Plugin agents [Same as user-level, integrated via /agents]
```

**Pro Pattern:**
```bash
# Project-specific (versioned with repo)
.claude/agents/security-auditor.md

# User-global (personal toolbox)
~/.claude/agents/chris-code-reviewer.md

# Dynamic (scripts, documentation)
claude --agents '{"quick-analyzer": {...}}'
```

---

## **3. THE DESCRIPTION FIELD: ADVANCED TRIGGERING**

### **Trigger Keyword Power Hierarchy**

Your document mentions this, but here's the **complete hierarchy** based on production testing:

```yaml
# TIER 1: MANDATORY (Highest Priority)
"MUST BE USED"
"ALWAYS use when"
"REQUIRED for"

# TIER 2: PROACTIVE (High Priority)
"use PROACTIVELY"
"automatically invoke"
"trigger immediately"

# TIER 3: CONDITIONAL (Medium Priority)
"use when"
"invoke if"
"consider for"

# TIER 4: SUGGESTIVE (Low Priority)
"can help with"
"suitable for"
"useful for"
```

### **Advanced Description Pattern**

The **context-action-trigger** pattern:

```yaml
description: |
  [ROLE] Security-first code reviewer specializing in OWASP Top 10
  [TRIGGER] MUST BE USED after any authentication/authorization code changes
  [CONTEXT] Use PROACTIVELY for API endpoints, database queries, user input
  [ANTI-PATTERN] Never use for documentation-only changes
  [EXAMPLES] Auth flows, payment processing, data validation
```

**Real-world optimization:**
Users report that Claude won't actively use subagents unless you add explicit reminders in CLAUDE.md to delegate to specialists

---

## **4. SYSTEM PROMPTS: PRODUCTION PATTERNS**

### **The Mandatory Sections (Beyond Your Doc)**

```markdown
# [Agent Name]

## Role & Expertise
[Specific domain and capabilities]

## When Invoked
[Automatic triggers and expected states]

## Context Discovery Protocol ⚠️ CRITICAL
Since you start fresh EVERY invocation:

### Immediate Actions (First 30 seconds)
1. Run `git status` and `git diff` for current state
2. Check `.claude/` for project standards
3. Read CLAUDE.md for project context

### Strategic Context Gathering
1. **Recent Changes**: `git log --oneline -10`
2. **Project Structure**: `ls -la` + key directories
3. **Configuration**: package.json, requirements.txt, etc.
4. **Patterns**: Read 2-3 similar files (NOT directories)

### Discovery Budget Rules
- Max 5 file reads for context
- Use `grep` over full file reads
- Target specific, not comprehensive
- Prioritize changed files over unchanged

## Execution Methodology
[Step-by-step workflow with decision trees]

## Output Protocol
### Success Format
```json
{
  "status": "READY_FOR_[NEXT_STAGE]",
  "summary": "2-3 sentence outcome",
  "artifacts": ["file1.ts", "file2.ts"],
  "handoff_context": {
    "decisions_made": ["choice1", "choice2"],
    "assumptions": ["assumption1"],
    "next_actions": ["action1", "action2"]
  }
}
```

### Failure Format
```json
{
  "status": "BLOCKED",
  "blocker": "Clear description",
  "required_info": ["what's needed"],
  "suggested_resolution": "actionable steps"
}
```

## Quality Gates
[Specific, measurable success criteria]

## Limitations & Boundaries
[What this agent explicitly does NOT do]
```

---

## **5. MCP INTEGRATION: THE GAME CHANGER**

### **Critical Understanding**

MCP (Model Context Protocol) servers give subagents "superpowers" - specialized, real-time information and tools

### **Configuration Patterns**

```yaml
# Global MCP (all agents inherit)
~/.claude.json

# Project-specific MCP (checked into repo)
.mcp.json

# Per-subagent MCP (selective access)
---
tools: Read, Write, mcp__context7__query, mcp__brave__search
---
```

### **Popular MCP Servers for Subagents**

**Essential:**
- **context7** - Versioned API documentation (fixes outdated LLM knowledge)
- **brave-search** - Web search (better than built-in for subagents)
- **puppeteer** - Browser automation and screenshots
- **github** - GitHub API access (issues, PRs, reviews)

**Advanced:**
- **supabase** - Database access (separate dev/prod instances)
- **linear** - Project management integration
- **sentry** - Error tracking and debugging
- **Memory MCP** - Persistent state between invocations

### **Critical MCP Issue**

There's an open feature request for MCP tools to be available ONLY to subagents (not main agent), preventing context pollution

**Current Workaround:**
```yaml
# In CLAUDE.md
## MCP Tool Usage Rules
- Main agent: NEVER directly use playwright/puppeteer tools
- Main agent: Delegate to ui-testing-agent for browser automation
- Subagents: Full MCP access as needed
```

---

## **6. HOOKS: THE SECRET ORCHESTRATION LAYER**

### **What Hooks Actually Do**

Hooks attach shell commands to lifecycle events and print to STDOUT, allowing you to surface next steps in the Claude transcript

### **Hook Event Types**

```json
{
  "hooks": {
    "PreToolUse": [/* Before tool execution */],
    "PostToolUse": [/* After tool completes */],
    "SubagentStop": [/* When subagent finishes */],
    "Stop": [/* When main agent stops */],
    "OnNotification": [/* When Claude sends notifications */],
    "OnSubmit": [/* Before Claude processes prompt */]
  }
}
```

### **Production Hook Patterns**

**1. Workflow Orchestration (PubNub Pattern)**

A SubagentStop hook reads a queue file and prints the next explicit command (e.g., "Use the architect-review subagent on 'use-case-presets'")

```bash
# .claude/hooks/workflow_coordinator.sh
#!/bin/bash
QUEUE_FILE=".claude/agent_queue.json"

if [ -f "$QUEUE_FILE" ]; then
  NEXT_TASK=$(jq -r '.next_task // empty' "$QUEUE_FILE")
  if [ -n "$NEXT_TASK" ]; then
    echo "🔄 NEXT: $NEXT_TASK"
    # Update queue
    jq '.next_task = null' "$QUEUE_FILE" > tmp && mv tmp "$QUEUE_FILE"
  fi
fi
```

**2. Security Gates**

```bash
# Block dangerous operations
#!/bin/bash
if echo "$CLAUDE_TOOL_ARGS" | grep -E "(DROP|DELETE|TRUNCATE)" > /dev/null; then
  echo "❌ BLOCKED: Destructive SQL operation"
  exit 1
fi
```

**3. Context Injection**

```bash
# Add dynamic context before prompt processing
#!/bin/bash
echo "📊 Current sprint: $(cat .sprint_info)"
echo "🔧 Recent errors: $(tail -5 error.log | wc -l) in last 5 entries"
```

### **Prompt-Based Hooks (Advanced)**

Instead of bash commands, use LLM (Haiku) to make intelligent, context-aware decisions

```json
{
  "hooks": {
    "Stop": [{
      "hooks": [{
        "type": "prompt",
        "prompt": "Evaluate if all tasks complete: $ARGUMENTS. Check git status, test results, and documentation. Return JSON with decision: 'approve' or 'block'"
      }]
    }]
  }
}
```

---

## **7. TOOL PERMISSIONS: GRANULAR SECURITY**

### **Tool Categories by Risk Level**

```yaml
# READ-ONLY (Reviewers, Analyzers)
tools: Read, Grep, Glob
# Risk: Minimal | Use: 70% of agents

# SEARCH-ENHANCED (Researchers)  
tools: Read, Grep, Glob, WebFetch, WebSearch
# Risk: Low | Use: Research, documentation

# WRITERS (Implementers)
tools: Read, Write, Edit, Bash, Grep, Glob
# Risk: Medium | Use: Code generation, file creation

# FULL ACCESS (Dangerous - use sparingly)
tools: # Omit field entirely = inherit all
# Risk: HIGH | Use: Orchestrators only

# MCP-SPECIFIC (Domain Tools)
tools: Read, mcp__github__create_issue, mcp__linear__add_ticket
# Risk: Varies | Use: Integration agents
```

### **Advanced Permission Pattern**

```yaml
# Security Auditor - Read-only with search
---
name: security-auditor
tools: Read, Grep, Glob, WebSearch
---

# Code Implementer - Write but NO Bash
---  
name: safe-implementer
tools: Read, Write, Edit, Grep, Glob
# Prevents: Dangerous shell commands, rm -rf, etc.
---

# Test Runner - Full access including Bash
---
name: test-runner  
tools: Read, Write, Edit, Bash, Grep, Glob
# Justified: Needs to run test commands
---
```

---

## **8. ORCHESTRATION PATTERNS: SEQUENTIAL VS PARALLEL**

### **When to Use Sequential (Your Doc is Correct)**

**Dependency Chain Pattern:**
```
requirements-analyst → architect → implementer → tester → reviewer
```

**Quality Gate Pattern:**
```
code-generator → security-scan → [GATE] → deployment
```

### **When to Use Parallel (Advanced)**

For large-scale refactoring, have a primary agent grep for all instances, then spin up a dedicated subagent for each file to perform replacement in a small, safe context

**Independent Domain Pattern:**
```
frontend-specialist ┐
backend-specialist  ├──→ integration-tester
database-optimizer  ┘
```

**Research Pattern:**
```
documentation-analyzer ┐
codebase-explorer      ├──→ architecture-synthesizer
dependency-auditor     ┘
```

### **The Parallelism Limit**

Claude Code supports up to 10 parallel tasks running concurrently, with intelligent queuing for 100+ tasks

**Advanced Parallel Script:**
```bash
# For massive refactoring
for dir in src/*; do
  claude -p "In $dir change all refs from oldFunc to newFunc" &
done
wait
```

---

## **9. COMMON MISTAKES & DEBUGGING**

### **Top 10 Pitfalls (From Production Teams)**

Setting up subagents can feel like wrangling very smart, very powerful, but sometimes very stubborn cats

**1. Model Incompatibility**
- **Issue:** Using Claude 3.7 on Bedrock (doesn't support parallel execution)
- **Fix:** Use Sonnet 4 or Opus 4

**2. Context Pollution**
Fix by running /clear often inside each worktree session
- **Issue:** Long conversations degrade quality
- **Fix:** Use /compact or /clear regularly

**3. Over-Tooling**
- **Issue:** Giving every agent access to every tool
- **Fix:** Minimum necessary permissions principle

**4. Vague Descriptions**
- **Issue:** "Helps with code" doesn't trigger automatically
- **Fix:** Use PROACTIVELY, MUST BE USED keywords

**5. Forgetting Clean Slate**
- **Issue:** Assuming agent remembers previous context
- **Fix:** Explicit Context Discovery section in every agent

**6. No Human Oversight**
Never replace human review with only subagents
- **Issue:** Letting agents run completely autonomous
- **Fix:** Quality gates, approval hooks, human checkpoints

**7. CLAUDE.md Bloat**
- **Issue:** 100KB+ CLAUDE.md files slow everything
- **Fix:** Keep under 25KB, use Skills for reusable knowledge

**8. Too Many Agents**
- **Issue:** Creating 50+ specialized agents
- **Fix:** Start with 5-10 core agents, expand based on actual need

**9. No Output Structure**
- **Issue:** Agents return unstructured prose
- **Fix:** Require JSON or structured markdown output

**10. Ignoring Git**
There isn't the same checkpoint system as other AI tools - Git is your safety net
- **Issue:** No rollback capability
- **Fix:** Commit frequently, use git worktrees

---

## **10. ADVANCED PATTERNS FROM PRODUCTION TEAMS**

### **Pattern 1: Hierarchical Orchestration**

Advanced practitioners implement hierarchical subagent structures where a master coordinator manages multiple specialized workers

```
project-orchestrator (Master)
├── planning-phase-coordinator
│   ├── requirements-analyst
│   ├── architect  
│   └── risk-assessor
├── implementation-coordinator
│   ├── frontend-dev
│   ├── backend-dev
│   └── database-engineer
└── quality-coordinator
    ├── test-automator
    ├── security-auditor
    └── performance-optimizer
```

### **Pattern 2: Domain Specialist Network**

Advanced patterns leverage up to 10 concurrent agents, each focused on specific architectural layers

```yaml
# Microservices Architecture Team
---
service-architect:
  focus: "Inter-service communication, data consistency"
  
security-engineer:
  focus: "OAuth flows, API gateway security"
  
infrastructure-specialist:
  focus: "Container orchestration, service mesh"
  
observability-engineer:
  focus: "Monitoring, tracing, alerting"
---
```

### **Pattern 3: The "Master-Clone" Architecture**

Put all context in CLAUDE.md, then let the main agent decide when and how to delegate work to copies of itself using Task(...)

**Controversial Take:** Some experts prefer this over custom subagents because:
- Dynamic delegation (agent decides)
- No brittle handoff protocols
- Shared base context from CLAUDE.md
- Simpler maintenance

### **Pattern 4: Skills + Subagents Combo**

Use Skills to teach expertise that any agent can apply; use subagents for independent task execution with specific tool permissions

```
Skill: security-review-checklist.md (knowledge)
  ↓ powers
Subagent: security-auditor.md (execution)
```

---

## **11. SUBAGENT RESUMPTION (Advanced Feature)**

Each subagent execution gets a unique agentId. Agent transcripts are stored in agent-{agentId}.jsonl files

**Use Cases:**
- Multi-step workflows across sessions
- Long-running research continued later
- Iterative debugging with context

**Example:**
```bash
# First session - agent returns ID: abc123
claude> Use code-analyzer to investigate auth bug

# Later session - resume that exact agent
claude> Resume agent abc123 and check error handling
```

---

## **12. INTEGRATION WITH OTHER TOOLS**

### **GitHub Actions Integration**

GHA logs are full agent logs. Teams review these at company level for common mistakes, bash errors, or unaligned practices - creates a data-driven flywheel

```yaml
name: AI Code Review
on: [pull_request]
jobs:
  review:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v3
      - name: AI Review
        run: |
          claude -p "Use security-auditor and code-reviewer on this PR"
```

### **VS Code Extension**

Using Claude Code within VS Code (with official plugin) allows opening Claude Code faster and seeing code changes within the editor

### **Combination with Other Tools**

Run o3-pro in Cursor alongside Claude Code. o3-pro is ridiculously good at planning - it can tell you EXACTLY what needs to change with surgical precision

**Optimal Workflow:**
1. o3-pro: Deep analysis, planning, system design
2. Claude Code: Implementation with subagents
3. GitHub Copilot: Inline completions, simple fixes

---

## **13. COST & PERFORMANCE OPTIMIZATION**

### **Token Consumption Reality**

Multi-agent workflows consume approximately 3-4x more tokens than traditional single-threaded interactions

**Optimization Strategies:**

1. **Context Caching**
Teams report 40-60% cost savings through intelligent agent orchestration
   - Share common context between related agents
   - Use Skills for reusable knowledge
   - Cache project structure, standards

2. **Dynamic Agent Spawning**
   - Simple tasks: Main agent only
   - Medium complexity: 2-3 agents
   - Complex: 5-8 agents maximum

3. **Automated Summarization**
   - Each agent returns concise summaries
   - Not full conversation histories

4. **Strategic Model Selection**
```yaml
# Expensive but accurate
model: opus

# Balanced  
model: sonnet  # Most common choice

# Fast and cheap
model: haiku  # Use for Explore subagent
```

---

## **14. THE FUTURE: SKILLS, PLUGINS, AND EVOLUTION**

### **Skills vs Subagents** (New Feature - October 2025)

Skills are portable and reusable knowledge; subagents are purpose-built for specific workflows with tool permissions and context isolation

**When to use each:**
- **Skill:** Reusable knowledge across agents/projects (e.g., "security-review-methodology")
- **Subagent:** Executable specialist with tools (e.g., "security-auditor")

### **Plugin System**

Plugins can provide custom subagents that integrate seamlessly with Claude Code. Plugin agents appear in /agents interface

**Install Pattern:**
```bash
/plugin install security-scanning
# Automatically adds security-auditor subagent
```

### **Community Ecosystem**

Major repositories:
- **VoltAgent/awesome-claude-code-subagents**: 100+ production-ready agents
- **wshobson/agents**: 85 specialized agents across 63 plugins
- **lst97/claude-code-sub-agents**: Full-stack development focus

---

## **15. CHRIS'S SPECIFIC USE CASE: PHEROMIND & EPISTEMIC SYMMETRY**

Given your background, here's how to architect subagents for your multi-agent system:

### **Recommended Agent Structure for Pheromind**

```yaml
# Orchestrator Layer
pheromind-orchestrator:
  role: "Meta-coordinator for autonomous dev pipeline"
  tools: Read, Task, Grep
  delegates_to: [sparc-planner, bmo-verifier, neo4j-architect]

# Planning Layer  
sparc-planner:
  role: "SPARC methodology implementation"
  phases: [Specification, Pseudocode, Architecture, Refinement, Completion]
  
epistemic-reasoner:
  role: "Apply systematic reasoning to problem decomposition"
  uses: "Epistemic symmetry principles"

# Execution Layer
knowledge-weaver:
  role: "Neo4j graph operations"
  tools: Read, Write, Edit, mcp__neo4j__query
  
bmo-verifier:
  role: "Behavior-Model-Oracle verification"
  tools: Read, Bash, Grep

# Analysis Layer  
universal-analyzer:
  role: "Codebase analysis with SAPIRE"
  output: "Reusable optimization framework"
```

### **TPG Meeting Prep Pattern**

For enterprise positioning:

```yaml
research-coordinator:
  delegates:
    - competitive-intel-agent (explainable AI market)
    - technical-writer-agent (white papers)
    - presentation-designer (executive decks)
    - demo-builder (proof-of-concept)
```

---

## **16. CRITICAL SUCCESS FACTORS**

Based on all research, **these are non-negotiable**:

1. **Start Small** - 3-5 agents maximum initially
2. **Context Discovery** - Every agent MUST have explicit discovery protocol
3. **Structured Outputs** - JSON or markdown with status fields
4. **Tool Minimalism** - Least privilege principle
5. **Trigger-Rich Descriptions** - PROACTIVELY, MUST BE USED
6. **Version Control** - `.claude/agents/` in repo
7. **Human Gates** - Approval hooks for destructive operations
8. **CLAUDE.md** - Clear delegation instructions
9. **Regular /clear** - Prevent context pollution
10. **Monitor Costs** - Track token usage per workflow

---

## **17. RESOURCES FOR DEEPER LEARNING**

**Official:**
- [Claude Code Subagents Docs](https://docs.claude.com/en/docs/claude-code/sub-agents)
- [Hooks Reference](https://docs.claude.com/en/docs/claude-code/hooks)
- [Best Practices](https://www.anthropic.com/engineering/claude-code-best-practices)

**Community:**
- [ClaudeLog](https://claudelog.com) - Comprehensive guides
- [PubNub's Guide](https://www.pubnub.com/blog/best-practices-for-claude-code-sub-agents/)
- [Zach Wills' Parallelization Guide](https://zachwills.net/how-to-use-claude-code-subagents-to-parallelize-development/)

**Repositories:**
- [awesome-claude-code-subagents](https://github.com/VoltAgent/awesome-claude-code-subagents)
- [wshobson/agents](https://github.com/wshobson/agents)

This should give you everything you need to master Claude Code subagents!