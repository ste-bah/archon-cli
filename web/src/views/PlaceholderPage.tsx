import { StatusPill } from "../components/StatusPill";

interface ActionItem {
  label: string;
  detail: string;
}

interface PlaceholderPageProps {
  eyebrow: string;
  title: string;
  summary: string;
  actions: ActionItem[];
}

export function PlaceholderPage({
  eyebrow,
  title,
  summary,
  actions,
}: PlaceholderPageProps) {
  return (
    <section className="panel panel--wide">
      <div className="panel-heading">
        <div>
          <span className="eyebrow">{eyebrow}</span>
          <h3>{title}</h3>
        </div>
        <StatusPill>foundation stub</StatusPill>
      </div>
      <p className="summary">{summary}</p>
      <div className="action-grid">
        {actions.map((action) => (
          <button key={action.label} className="action-button" type="button" disabled>
            <strong>{action.label}</strong>
            <span>{action.detail}</span>
          </button>
        ))}
      </div>
    </section>
  );
}
