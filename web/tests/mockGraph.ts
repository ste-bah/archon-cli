export function graphNodes() {
  return [
    node("docs", "Docs", "source", "Filesystem documents and PDFs", 42),
    node("kb", "Knowledge base", "source", "Compiled /kb material", 8),
    node("chunks", "Chunks", "derived", "Searchable document spans", 120),
    node("claims", "Claims", "evidence", "Extracted or cited assertions", 80),
    node("evidence", "Evidence", "evidence", "Provenance and citations", 55),
    node("memory", "Memory", "learning", "Persistent recall rows", 20),
    node("learning", "LearningEvents", "learning", "Governed learning rows", 12),
    node("reasoning", "Reasoning quality", "reasoning", "Claim and correction events", 9),
    node("world", "World model", "model", "Trace rows and candidates", 4),
    node("sessions", "Sessions", "runtime", "Agent turns and activity ledgers", 14),
    node("pipelines", "Pipelines", "runtime", "Stages and agents", 6),
    node("artifacts", "Artifacts", "output", "Reports and bundles", 18),
  ];
}

export function graphEdges() {
  return [
    edge("docs_chunks", "docs", "chunks", "ingested as"),
    edge("kb_chunks", "kb", "chunks", "compiled into"),
    edge("chunks_claims", "chunks", "claims", "support"),
    edge("claims_evidence", "claims", "evidence", "grounded by"),
    edge("evidence_memory", "evidence", "memory", "stored as"),
    edge("memory_learning", "memory", "learning", "feeds"),
    edge("sessions_reasoning", "sessions", "reasoning", "emits"),
    edge("reasoning_world", "reasoning", "world", "adds trace signal"),
    edge("sessions_pipelines", "sessions", "pipelines", "runs"),
    edge("pipelines_artifacts", "pipelines", "artifacts", "writes"),
  ];
}

function node(id: string, label: string, kind: string, detail: string, count: number) {
  return { id, label, kind, detail, count };
}

function edge(id: string, source: string, target: string, label: string) {
  return { id, source, target, label };
}
