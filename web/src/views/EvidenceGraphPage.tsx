import cytoscape, { type Core, type ElementDefinition } from "cytoscape";
import { useEffect, useMemo, useRef, useState } from "react";
import { StatusPill } from "../components/StatusPill";
import type { EvidenceGraphNode, EvidenceGraphSummary } from "../api/generated/web";
import "./EvidenceGraphPage.css";

interface EvidenceGraphPageProps {
  graph?: EvidenceGraphSummary;
}

const positions: Record<string, { x: number; y: number }> = {
  docs: { x: 80, y: 120 },
  kb: { x: 80, y: 300 },
  chunks: { x: 260, y: 210 },
  claims: { x: 440, y: 210 },
  evidence: { x: 620, y: 210 },
  memory: { x: 800, y: 120 },
  learning: { x: 980, y: 120 },
  sessions: { x: 800, y: 340 },
  reasoning: { x: 980, y: 260 },
  world: { x: 1160, y: 260 },
  pipelines: { x: 1160, y: 420 },
  artifacts: { x: 1340, y: 420 },
};

export function EvidenceGraphPage({ graph }: EvidenceGraphPageProps) {
  const containerRef = useRef<HTMLDivElement | null>(null);
  const cyRef = useRef<Core | null>(null);
  const [selectedId, setSelectedId] = useState("evidence");
  const selected = useMemo(
    () => graph?.nodes.find((node) => node.id === selectedId) ?? graph?.nodes[0],
    [graph, selectedId],
  );

  useEffect(() => {
    if (!containerRef.current || !graph) {
      return;
    }
    const cy = cytoscape({
      container: containerRef.current,
      elements: toElements(graph),
      layout: { name: "preset", fit: true, padding: 28 },
      minZoom: 0.45,
      maxZoom: 2,
      style: graphStyles,
    });
    cy.on("tap", "node", (event) => setSelectedId(event.target.id()));
    cyRef.current = cy;
    return () => {
      cy.destroy();
      cyRef.current = null;
    };
  }, [graph]);

  useEffect(() => {
    cyRef.current?.nodes().removeClass("selected");
    cyRef.current?.getElementById(selectedId).addClass("selected");
  }, [selectedId, graph]);

  return (
    <section className="evidence-layout">
      <div className="panel graph-panel">
        <div className="panel-heading">
          <div>
            <span className="eyebrow">Graph</span>
            <h3>Evidence graph</h3>
          </div>
          <StatusPill tone={graph?.degraded ? "warn" : "good"}>
            {graph?.degraded ? "clustered" : "direct render"}
          </StatusPill>
        </div>
        <p className="summary">
          Follow how documents, chunks, claims, evidence, memories, reasoning-quality
          rows, world-model traces, pipelines, and artifacts connect.
        </p>
        <div ref={containerRef} className="evidence-graph" aria-label="Evidence relationship graph" />
      </div>
      <aside className="panel graph-details">
        <span className="eyebrow">Selection</span>
        <h3>{selected?.label ?? "Loading graph"}</h3>
        <p>{selected?.detail ?? "Waiting for graph data from the local API."}</p>
        <div className="graph-facts">
          <GraphFact label="Kind" value={selected?.kind ?? "loading"} />
          <GraphFact label="Rows" value={String(selected?.count ?? 0)} />
          <GraphFact label="Nodes" value={String(graph?.nodes.length ?? 0)} />
          <GraphFact label="Edges" value={String(graph?.edges.length ?? 0)} />
          <GraphFact label="Budget" value={`${graph?.nodeBudget ?? "-"} / ${graph?.edgeBudget ?? "-"}`} />
        </div>
        <div className="graph-edge-list">
          {(graph?.edges ?? [])
            .filter((edge) => edge.source === selected?.id || edge.target === selected?.id)
            .map((edge) => (
              <div key={edge.id} className="graph-edge-row">
                <strong>{edge.label}</strong>
                <span>
                  {edge.source} {"->"} {edge.target}
                </span>
              </div>
            ))}
        </div>
      </aside>
    </section>
  );
}

function GraphFact({ label, value }: { label: string; value: string }) {
  return (
    <div className="graph-fact">
      <span>{label}</span>
      <strong>{value}</strong>
    </div>
  );
}

function toElements(graph: EvidenceGraphSummary): ElementDefinition[] {
  return [
    ...graph.nodes.map((node) => ({
      data: node,
      position: positions[node.id] ?? fallbackPosition(node),
      classes: node.kind,
    })),
    ...graph.edges.map((edge) => ({ data: edge })),
  ];
}

function fallbackPosition(node: EvidenceGraphNode) {
  const seed = [...node.id].reduce((sum, char) => sum + char.charCodeAt(0), 0);
  return { x: 140 + (seed % 6) * 180, y: 120 + (seed % 4) * 110 };
}

const graphStyles: cytoscape.StylesheetJson = [
  {
    selector: "node",
    style: {
      label: "data(label)",
      "background-color": "#87d8b4",
      color: "#e8ece8",
      "font-size": "12px",
      "text-valign": "center",
      "text-halign": "center",
      "text-wrap": "wrap",
      "text-max-width": "92px",
      width: "86px",
      height: "54px",
      shape: "round-rectangle",
      "border-width": "1px",
      "border-color": "#303838",
    },
  },
  { selector: "node.source", style: { "background-color": "#6fb7e8" } },
  { selector: "node.learning", style: { "background-color": "#87d8b4" } },
  { selector: "node.reasoning", style: { "background-color": "#f4c471" } },
  { selector: "node.model", style: { "background-color": "#b79df2" } },
  { selector: "node.runtime", style: { "background-color": "#ea8d8d" } },
  { selector: "node.output", style: { "background-color": "#9ec27f" } },
  {
    selector: "node.selected",
    style: { "border-width": "4px", "border-color": "#ffffff" },
  },
  {
    selector: "edge",
    style: {
      label: "data(label)",
      color: "#aab4b0",
      "curve-style": "bezier",
      "font-size": "10px",
      "line-color": "#5d6a66",
      "target-arrow-color": "#5d6a66",
      "target-arrow-shape": "triangle",
      width: "2px",
    },
  },
];
