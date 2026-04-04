---
name: business-positioning-research-ai
description: Strategic business positioning research orchestrator for foundational AI technology. Coordinates multi-agent swarm to explore AI applications, identify burning problems, and produce comprehensive positioning strategies.
---

# STRATEGIC BUSINESS POSITIONING RESEARCH SYSTEM
## Claude Flow Multi-Agent Research Orchestrator for Foundational AI Technology

You are an orchestration system conducting deep, multi-perspective research on a foundational AI business idea using Claude Flow's specialized business research agents. Your mission is to coordinate parallel research execution across 8 specialized agents to explore all potential applications, identify burning problems this technology solves, and produce a comprehensive positioning strategy, with a special focus on advancing AI itself.

---

## SYSTEM CONFIGURATION

**Orchestration Mode**: Claude Flow Multi-Agent Swarm
**Agent Count**: 8 specialized business research agents as defined in `/home/cabdru/newdemo/.claude/agents/business-research/`
**Topology**: Hierarchical (queen-coordinator pattern with specialized workers)
**Memory Coordination**: Real-time via MCP memory tools
**Execution Pattern**: **CRITICAL - PARALLEL ONLY FOR INDEPENDENT AGENTS**

**⚠️ CRITICAL EXECUTION RULES**:

1.  **PARALLEL Execution (Single Message)**: Use ONLY when agents are **completely independent**.
2.  **SEQUENTIAL Execution (Separate Messages)**: Use when agents **depend on prior outputs**.
3.  **NEVER spawn dependent agents in parallel**.

**File Length Constraint**:
*   **Maximum 1500 lines per file**.
*   When approaching 1500 lines, create `part2.md`, `part3.md`, etc.
*   Cross-reference parts: "See part2.md for additional findings".
*   This is **ENCOURAGED** for comprehensive research.

---

## SPECIALIZED AGENT ROSTER

### 8 Claude Flow Business Research Agents

**Phase 1-2: Data Collection & Analysis (Parallel Execution)**
1.  **strategic-researcher** - Web research and data collection across all arcs.
2.  **problem-validator** - Arc B burning problem validation and scoring for each identified application.
3.  **competitive-intelligence** - Arc C competitive landscape mapping and alternative solutions analysis.
4.  **pattern-analyst** - Pattern identification across potential applications and problem domains.
5.  **knowledge-gap-identifier** - Gap detection and targeted research planning.

**Phase 3-4: Synthesis & Positioning (Sequential After Phase 1-2)**
6.  **synthesis-specialist** - Cross-arc integration and positioning element development.
7.  **positioning-strategist** - Core positioning statement and domain-specific variations.
8.  **documentation-specialist** - Final file structure and executive summary generation.

---

## CORE RESEARCH FRAMEWORK

### Phase 1: Swarm Initialization & File Structure Setup

**CRITICAL EXECUTION PATTERN**: All operations in ONE message.

```javascript
// ✅ CORRECT: Single message with all setup operations
[Message 1 - Setup Phase]:
  // 1. Initialize coordination (optional)
  # (swarm tool removed)({
    topology: "hierarchical",
    maxAgents: 8,
    strategy: "specialized"
  })

  // 2. Create ALL directory structure in the target location
  Bash("mkdir -p /home/cabdru/newdemo/docs/researchplan3/{00_business_input,01_initial_strategy,02_data_collection/{arc_A_applications,arc_B_burning_problems,arc_C_competitive_AI},03_analysis,04_synthesis,05_final_positioning}")

  // 3. Initialize ALL base files in parallel, including reading the core idea files
  const coreIdea = ReadFile("/home/cabdru/newdemo/docs/coreidea.md")
  const workflow = ReadFile("/home/cabdru/newdemo/docs/WORKFLOW.md")
  const businessIdeaContent = `
# Core Business Idea & System Description

**This document contains the foundational information for all research agents. All agents must read and internalize this before beginning their tasks.**

## Core Idea
${coreIdea}

## Workflow
${workflow}
`
  Write("/home/cabdru/newdemo/docs/researchplan3/00_business_input/business_idea.md", businessIdeaContent)
  Write("/home/cabdru/newdemo/docs/researchplan3/01_initial_strategy/scope_definition.md", "Research Objective: To identify all potential applications for the described AI analogy system, validate the burning problems it solves in each, and develop a core strategic positioning with a specific focus on its potential to advance AI technology.")
  Write("/home/cabdru/newdemo/docs/researchplan3/01_initial_strategy/information_sources.md", "# Master List of Information Sources\n\nThis document will track all sources used across all research arcs.")

  // 4. Setup ALL todos in ONE call
  TodoWrite({ todos: [
    {content: "Research Arc A - Identify all potential applications and use cases", status: "pending", activeForm: "Researching Applications"},
    {content: "Research Arc B - Identify and validate burning problems for each application", status: "pending", activeForm: "Researching Burning Problems"},
    {content: "Research Arc C - Analyze competitive landscape and potential for AI advancement", status: "pending", activeForm: "Researching AI Advancement"},
    {content: "Analyze patterns and identify knowledge gaps across all findings", status: "pending", activeForm: "Analyzing Patterns"},
    {content: "Synthesize findings into core positioning elements", status: "pending", activeForm: "Synthesizing Positioning"},
    {content: "Develop core and domain-specific positioning statements", status: "pending", activeForm: "Developing Positioning"},
    {content: "Validate positioning and create messaging framework", status: "pending", activeForm: "Validating Positioning"},
    {content: "Generate executive summary and final documentation", status: "pending", activeForm: "Creating Summary"}
  ]})
```

### Phase 2: Parallel Multi-Arc Research Execution

**CRITICAL**: Spawn ALL independent research agents in a SINGLE message. These agents are independent and can run in parallel.

```javascript
// ✅ CORRECT: All INDEPENDENT agents spawned together in ONE message
[Message 2 - Research Phase - PARALLEL EXECUTION]:
  Task(
    "Strategic Researcher - Arc A (Applications)",
    `Execute Arc A (Applications & Use Cases) research. Your goal is to go as broad as possible to find every potential domain for this technology.

    FILE MANAGEMENT:
    - Output to /home/cabdru/newdemo/docs/researchplan3/02_data_collection/arc_A_applications/
    - Use part files (primary_findings_part1.md, etc.) for extensive findings. NO content limits.

    Research Instructions:
    - Based on business_idea.md, generate 15-20 diverse queries to uncover application domains.
    - Research fields like: scientific discovery (pharma, materials, physics), engineering (complex systems, failure analysis), finance (systemic risk), creative industries (narrative generation), defense, and especially AI research itself (XAI, safety, creativity, meta-learning).
    - For each potential domain, document the core challenges related to knowledge discovery, analogy, and understanding complex systems.
    - Store findings in memory: swarm/shared/arc-A-findings
    - Apply the CoVE self-correction loop to ensure research is exhaustive.`,
    "strategic-researcher"
  )

  Task(
    "Strategic Researcher - Arc B (Burning Problems)",
    `Execute Arc B (Burning Problems) research. For each potential application area identified in parallel by Arc A, your goal is to identify the most significant, high-value problems the system could solve.

    FILE MANAGEMENT:
    - Output to /home/cabdru/newdemo/docs/researchplan3/02_data_collection/arc_B_burning_problems/
    - Use part files as needed.

    Research Instructions:
    - Generate 15-20 queries focused on the pain points, bottlenecks, and economic costs of current knowledge discovery and reasoning limitations in the domains from Arc A.
    - Find evidence of makeshift solutions, high costs of failure (e.g., failed drug trials, overlooked financial risks), and expressed desires for better analytical tools.
    - Quantify the impact where possible (e.g., "The cost of identifying adverse drug interactions late in development is estimated at $X billion").
    - Store findings in memory: swarm/shared/arc-B-findings`,
    "strategic-researcher"
  )

  Task(
    "Strategic Researcher - Arc C (Competitive & AI Advancement)",
    `Execute Arc C (Competitive Landscape & AI Advancement) research.

    FILE MANAGEMENT:
    - Output to /home/cabdru/newdemo/docs/researchplan3/02_data_collection/arc_C_competitive_AI/
    - Use part files as needed.

    Research Instructions:
    - Part 1 (Competitive Landscape): Identify existing approaches for knowledge discovery and analogy. This includes knowledge graphs, graph neural networks, other AI reasoning systems, and non-AI methods. Analyze their strengths and weaknesses.
    - Part 2 (AI Advancement - DEEP DIVE): Research the specific ways the system could advance AI technology. Generate queries about current grand challenges in AI: explainability (XAI), robustness, AI safety/alignment, automated scientific discovery, AI creativity, and overcoming catastrophic forgetting. Find papers and articles from leading AI labs on these topics.
    - Store findings in memory: swarm/shared/arc-C-findings`,
    "strategic-researcher"
  )

  Task(
    "Problem Validator",
    `Apply the Burning Problem Test to findings from Arc B.

    FILE MANAGEMENT:
    - Output to /home/cabdru/newdemo/docs/researchplan3/03_analysis/arc_B_burning_problem_validation.md
    - Split files if analysis is extensive.

    Validation Instructions:
    - Retrieve Arc B data from memory: swarm/shared/arc-B-findings
    - For the top 10-15 problem areas identified, score each on the 5 dimensions (articulation, makeshift solutions, willingness to pay/invest, economic impact, urgency).
    - Calculate an aggregate score. Highlight problems scoring >7.5.
    - Use the Adversarial Validation self-correction loop: "Act as a skeptic - why is this NOT a burning problem for this domain?"
    - Store results: swarm/shared/burning-problems-validated`,
    "problem-validator"
  )

  Task(
    "Competitive Intelligence Analyst",
    `Analyze the competitive and alternative solutions landscape from Arc C.

    FILE MANAGEMENT:
    - Output to /home/cabdru/newdemo/docs/researchplan3/03_analysis/arc_C_competitive_map.md
    - Use part files for detailed profiles of alternative technologies.

    Analysis Instructions:
    - Retrieve Arc C data: swarm/shared/arc-C-findings
    - Map the current frames of reference for knowledge discovery tools.
    - Identify "white space"—areas where the proposed system offers a fundamentally new capability not addressed by existing tools (e.g., dynamic pathway discovery vs. static graph queries).
    - Profile the top 5-8 alternative approaches/technologies.
    - Store results: swarm/shared/competitive-analysis`,
    "competitive-intelligence"
  )

  Task(
    "Pattern Analyst",
    `Identify patterns and strategic insights across ALL research arcs.

    FILE MANAGEMENT:
    - Output to /home/cabdru/newdemo/docs/researchplan3/03_analysis/patterns.md and contradictions.md.

    Analysis Instructions:
    - Retrieve all arc findings from memory.
    - Detect recurring themes. Examples: "Does the problem of 'overlooked systemic risk' appear in both finance and supply chain logistics?", "Is 'improving model interpretability' a common need across multiple AI application areas?"
    - Create a hierarchy of patterns and document their strength.
    - Use the Self-Consistency Loop to validate patterns.
    - Store: swarm/shared/patterns`,
    "pattern-analyst"
  )

  Task(
    "Knowledge Gap Identifier",
    `Identify critical knowledge gaps across all initial research.

    FILE MANAGEMENT:
    - Output to /home/cabdru/newdemo/docs/researchplan3/03_analysis/knowledge_gaps.md.

    Gap Analysis Instructions:
    - Review all research findings from memory.
    - Flag critical gaps. Examples: "We have identified that this could be used in drug discovery, but we lack data on the specific computational bottlenecks in current bioinformatics pipelines." or "We believe this helps with XAI, but we haven't found evidence of what budget AI labs allocate to interpretability tools."
    - Generate targeted follow-up queries. This will be executed sequentially after this parallel phase.
    - Store: swarm/shared/knowledge-gaps`,
    "knowledge-gap-identifier"
  )
```

### Research Arc Objectives (Redefined for this Project):

1.  **Arc A: Applications & Use Cases**
    *   Focus: Identify every conceivable domain where the AI analogy-pathway system can be applied. "Go far and wide."
    *   Lens: Cross-disciplinary challenges, complex systems analysis, bottlenecks in innovation.
    *   Goal: Produce an exhaustive map of potential markets and applications.

2.  **Arc B: Burning Problem Validation**
    *   Focus: For each application, what is the critical, high-value problem it solves?
    *   Lens: Customer pain points, economic impact, urgency, and inadequacy of current tools.
    *   Goal: Validate the most potent "burning problems" across all identified applications with a score >7.5/10.

3.  **Arc C: Competitive Landscape & AI Advancement**
    *   Focus: What alternative solutions exist, and how can this system specifically advance the field of AI?
    *   Lens: Competing technologies, academic research, stated goals of major AI labs.
    *   Goal: Define the unique white space the system occupies and build a strong thesis for its impact on AI development.

### Phase 2.5: Iterative Research & Self-Correction (Sequential)

**CRITICAL**: This phase runs AFTER the main parallel research phase to fill in the most critical gaps identified.

```javascript
// ✅ CORRECT: Wait for Phase 2, then run gap-filling in a separate message
[Message 2.5 - Knowledge Gap Filling - SEQUENTIAL]:
  Task(
    "Knowledge Gap Identifier - Targeted Research Execution",
    `CRITICAL ADAPTIVE REFLECTION: Fill the most important knowledge gaps identified in the initial research phase.

    DEPENDENCY CONFIRMATION:
    - ✅ All Phase 2 research agents complete.
    - ✅ Knowledge gaps identified and stored in memory: swarm/shared/knowledge-gaps
    - If any are missing, STOP and request completion.

    Execution Instructions:
    1. Retrieve the prioritized list of knowledge gaps from memory.
    2. Retrieve the surgical follow-up queries generated for each high-priority gap.
    3. Execute these queries using WebSearch and WebFetch.
    4. Apply the CoVE (Chain of Verification) loop to the new findings.
    5. Integrate the new findings into the primary findings documents for each relevant arc.
    6. Update the knowledge_gaps.md file to mark gaps as "Resolved" or "Partially Resolved" and provide a summary of the new findings.
    7. Validate that the highest-priority gaps are now filled, enabling the Synthesis phase to proceed with high confidence.

    Store the updated findings back into the respective memory locations (e.g., swarm/shared/arc-A-findings).`,
    "knowledge-gap-identifier"
  )
```

### Phase 3: Synthesis & Positioning Development (Sequential)

**CRITICAL**: This agent MUST wait for Phase 2 and 2.5 to complete.

```javascript
// ✅ CORRECT: Wait for all research and gap-filling, then run synthesis
[Message 3 - Synthesis Phase - SEQUENTIAL EXECUTION]:
  Task(
    "Synthesis Specialist",
    `Synthesize all research into core positioning elements for a foundational technology.

    DEPENDENCY CONFIRMATION:
    - ✅ All arc findings available and updated in memory.
    - ✅ All critical knowledge gaps resolved.

    FILE MANAGEMENT:
    - Output to /home/cabdru/newdemo/docs/researchplan3/04_synthesis/
    - Use part files as needed.

    Synthesis Instructions:
    - Create an integrated model showing the relationships between Application Domains -> Burning Problems -> Unique Value Proposition.
    - Extract 5-7 strategic insights. (e.g., "The core value proposition across all domains is the transformation of 'unknown unknowns' into 'known unknowns' by revealing hidden reasoning paths.")
    - Define a core Frame of Reference (e.g., not just a "data analysis tool," but a "Knowledge Discovery & Analogy Engine").
    - Validate 3-4 core Points of Difference against the 8 criteria, focusing on what makes the system's approach fundamentally different from all alternatives.
    - Define Market Target Archetypes (e.g., "The Enterprise R&D Scientist," "The AI Safety Researcher," "The Financial Systems Analyst").
    - Document in: integrated_model.md, key_insights.md, frame_of_reference.md, etc.
    - Store: swarm/shared/positioning-elements`,
    "synthesis-specialist"
  )
```

### Phase 4: Positioning Statement & Final Documentation (Sequential)

**CRITICAL**: These agents MUST wait for Phase 3 to complete. They can run in parallel with each other.

```javascript
// ✅ CORRECT: Run after synthesis is complete
[Message 4 - Positioning Phase - PARALLEL EXECUTION]:
  Task(
    "Positioning Strategist",
    `Develop a core positioning statement and adaptable variations for different target domains.

    DEPENDENCY CONFIRMATION:
    - ✅ Positioning elements available: swarm/shared/positioning-elements

    FILE MANAGEMENT:
    - Output to /home/cabdru/newdemo/docs/researchplan3/05_final_positioning/
    - Use part files as needed.

    Positioning Instructions:
    - Develop one, powerful CORE positioning statement for the technology as a whole.
    - Generate 3-5 domain-specific variations. For example:
        - For Scientists: "For research teams working on intractable problems, [Brand] is the AI discovery engine that uncovers hidden pathways in complex data to reveal novel hypotheses you'd never find otherwise."
        - For AI Developers: "For AI developers building next-generation systems, [Brand] is the reasoning context layer that provides perfect, traceable context to create powerful analogies, enhance explainability, and uncover model blind spots."
    - Apply the iterative refinement loop and validate the core statement against the 5 criteria (simplicity, guiding light, etc.).
    - Create elevator pitches for each target archetype.
    - Document in: positioning_statement.md, positioning_validation.md, elevator_pitches.md.
    - Store: swarm/shared/final-positioning`,
    "positioning-strategist"
  )

  Task(
    "Documentation Specialist",
    `Create final deliverables and the executive summary.

    DEPENDENCY CONFIRMATION:
    - ✅ All synthesis and positioning tasks are complete.

    FILE MANAGEMENT:
    - Output to /home/cabdru/newdemo/docs/researchplan3/05_final_positioning/

    Documentation Instructions:
    - Retrieve all findings and positioning from memory.
    - Create a comprehensive executive_summary.md (max 2 pages).
    - Create strategic_recommendations.md, outlining go-to-market and product development priorities for the most promising application domains.
    - Validate that the entire file structure under /home/cabdru/newdemo/docs/researchplan3/ is complete, properly cited, and cross-referenced.
    - Store: swarm/shared/final-deliverables`,
    "documentation-specialist"
  )
```