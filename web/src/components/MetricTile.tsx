interface MetricTileProps {
  label: string;
  value: string;
  detail: string;
}

export function MetricTile({ label, value, detail }: MetricTileProps) {
  return (
    <section className="metric-tile" aria-label={label}>
      <span className="metric-tile__label">{label}</span>
      <strong>{value}</strong>
      <span className="metric-tile__detail">{detail}</span>
    </section>
  );
}
