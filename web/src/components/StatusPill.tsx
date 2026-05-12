interface StatusPillProps {
  tone?: "good" | "warn" | "muted";
  children: React.ReactNode;
}

export function StatusPill({ tone = "muted", children }: StatusPillProps) {
  return <span className={`status-pill status-pill--${tone}`}>{children}</span>;
}
