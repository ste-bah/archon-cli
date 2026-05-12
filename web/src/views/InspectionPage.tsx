import { StatusPill } from "../components/StatusPill";

export interface FactRow {
  label: string;
  value: string;
  tone?: "good" | "warn" | "muted";
}

export interface ListRow {
  label: string;
  detail: string;
}

interface InspectionPageProps {
  eyebrow: string;
  title: string;
  summary: string;
  facts: FactRow[];
  rows: ListRow[];
}

export function InspectionPage({
  eyebrow,
  title,
  summary,
  facts,
  rows,
}: InspectionPageProps) {
  return (
    <section className="panel panel--wide">
      <div className="panel-heading">
        <div>
          <span className="eyebrow">{eyebrow}</span>
          <h3>{title}</h3>
        </div>
        <StatusPill tone="good">live data</StatusPill>
      </div>
      <p className="summary">{summary}</p>
      <div className="fact-grid">
        {facts.map((fact) => (
          <div key={fact.label} className="fact-row">
            <span>{fact.label}</span>
            <StatusPill tone={fact.tone ?? "muted"}>{fact.value}</StatusPill>
          </div>
        ))}
      </div>
      <div className="inspection-list">
        {rows.map((row) => (
          <div key={`${row.label}:${row.detail}`} className="inspection-row">
            <strong>{row.label}</strong>
            <span>{row.detail}</span>
          </div>
        ))}
      </div>
    </section>
  );
}
