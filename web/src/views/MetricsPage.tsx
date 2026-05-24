import { Activity, Database, Gauge, TimerReset } from "lucide-react";
import { useState } from "react";
import { StatusPill } from "../components/StatusPill";
import type { MetricsSummary, ProviderRuntimeMetric } from "../api/generated/web";
import "./MetricsPage.css";

interface MetricsPageProps {
  metrics?: MetricsSummary;
  liveCount?: number;
}

export function MetricsPage({ metrics, liveCount = 0 }: MetricsPageProps) {
  const [selected, setSelected] = useState<MetricSelection | undefined>();
  const stores = metrics?.stores ?? [];
  const performance = metrics?.performance ?? [];
  const queues = metrics?.queues ?? [];
  const events = metrics?.recentEvents ?? [];
  const providerMetrics = metrics?.providerMetrics ?? [];
  const providerEvents = metrics?.providerEvents ?? [];

  return (
    <section className="metrics-layout">
      <div className="panel panel--wide">
        <div className="panel-heading">
          <div>
            <span className="eyebrow">Health</span>
            <h3>Performance metrics</h3>
          </div>
          <StatusPill tone="good">{formatBytes(metrics?.webBundleBytes ?? 0)} bundle</StatusPill>
        </div>
        <div className="metrics-grid">
          <MetricCard
            icon={<Gauge size={18} aria-hidden="true" />}
            label="Bundle files"
            value={metrics?.webBundleFiles ?? 0}
            detail="embedded assets"
            onSelect={() =>
              setSelected(selection("Bundle files", `${metrics?.webBundleFiles ?? 0}`, `${formatBytes(metrics?.webBundleBytes ?? 0)} embedded assets`))
            }
          />
          <MetricCard
            icon={<Activity size={18} aria-hidden="true" />}
            label="Live events"
            value={liveCount}
            detail="current snapshot"
            onSelect={() => setSelected(selection("Live events", `${liveCount}`, "current web live-event snapshot"))}
          />
          <MetricCard
            icon={<Database size={18} aria-hidden="true" />}
            label="Providers"
            value={providerMetrics.length}
            detail="runtime telemetry"
            onSelect={() => setSelected(selection("Providers", `${providerMetrics.length}`, "runtime telemetry providers with recorded metrics"))}
          />
        </div>
      </div>

      {selected && (
        <section className="panel panel--wide metrics-selection" aria-label="Selected metric detail">
          <div>
            <span className="eyebrow">Selection</span>
            <h3>{selected.label}</h3>
          </div>
          <strong>{selected.value}</strong>
          <p>{selected.detail}</p>
        </section>
      )}

      <section className="panel">
        <div className="panel-heading">
          <h3>Performance targets</h3>
          <StatusPill>{performance.length} targets</StatusPill>
        </div>
        <div className="metrics-list">
          {performance.map((item) => (
            <MetricRow
              key={item.label}
              item={item}
              onSelect={() => setSelected(selection(item.label, item.value, `${item.unit} · ${item.status}`))}
            />
          ))}
        </div>
      </section>

      <section className="panel">
        <div className="panel-heading">
          <h3>Ledger activity</h3>
          <StatusPill>{queues.length} ledgers</StatusPill>
        </div>
        <div className="metrics-list">
          {queues.map((item) => (
            <MetricRow
              key={item.label}
              item={item}
              onSelect={() => setSelected(selection(item.label, item.value, `${item.unit} · ${item.status}`))}
            />
          ))}
        </div>
      </section>

      <section className="panel">
        <div className="panel-heading">
          <h3>Store health</h3>
          <StatusPill>{stores.length} stores</StatusPill>
        </div>
        <div className="metrics-list">
          {stores.map((store) => (
            <button
              key={store.path}
              type="button"
              className="metrics-row"
              onClick={() => setSelected(selection(store.label, store.status, `${store.path} · ${store.files} files · ${formatBytes(store.bytes)}`))}
            >
              <div>
                <strong>{store.label}</strong>
                <span>{store.path}</span>
                <small>
                  {store.files} files · {formatBytes(store.bytes)}
                </small>
              </div>
              <StatusPill tone={statusTone(store.status)}>{store.status}</StatusPill>
            </button>
          ))}
        </div>
      </section>

      <section className="panel">
        <div className="panel-heading">
          <h3>Provider runtime</h3>
          <StatusPill>{providerMetrics.length} providers</StatusPill>
        </div>
        <div className="metrics-list">
          {providerMetrics.length === 0 ? (
            <div className="metrics-empty">
              <TimerReset size={18} aria-hidden="true" />
              <span>No provider runtime telemetry found in the learning store.</span>
            </div>
          ) : (
            providerMetrics.map((provider) => (
              <ProviderMetricRow
                key={provider.providerId}
                provider={provider}
                onSelect={() =>
                  setSelected(
                    selection(
                      provider.providerId,
                      `${provider.requestCount} requests`,
                      `${provider.errorCount} errors · ${provider.retryCount} retries · ${provider.inputTokens + provider.outputTokens} tokens`,
                    ),
                  )
                }
              />
            ))
          )}
        </div>
      </section>

      <section className="panel">
        <div className="panel-heading">
          <h3>Provider event tail</h3>
          <StatusPill>{providerEvents.length} events</StatusPill>
        </div>
        <div className="metrics-event-list">
          {providerEvents.length === 0 ? (
            <div className="metrics-empty">
              <TimerReset size={18} aria-hidden="true" />
              <span>Provider events will appear after runtime calls are recorded.</span>
            </div>
          ) : (
            providerEvents.map((event) => (
              <button
                key={`${event.providerId}:${event.createdAt}`}
                type="button"
                className="metrics-event"
                onClick={() => setSelected(selection(event.providerId, event.eventType, event.message))}
              >
                <header>
                  <strong>{event.providerId}</strong>
                  <StatusPill tone={statusTone(event.severity)}>{event.severity}</StatusPill>
                </header>
                <p>{event.message}</p>
                <small>
                  {event.modelId} · {event.eventType} · {event.createdAt}
                </small>
              </button>
            ))
          )}
        </div>
      </section>

      <section className="panel">
        <div className="panel-heading">
          <h3>Recent event tail</h3>
          <StatusPill>{events.length} rows</StatusPill>
        </div>
        <div className="metrics-event-list">
          {events.length === 0 ? (
            <div className="metrics-empty">
              <TimerReset size={18} aria-hidden="true" />
              <span>No recent metric ledger events found.</span>
            </div>
          ) : (
            events.map((event) => (
              <button
                key={`${event.source}:${event.summary}`}
                type="button"
                className="metrics-event"
                onClick={() => setSelected(selection(event.source, event.severity, event.summary))}
              >
                <header>
                  <strong>{event.source}</strong>
                  <StatusPill tone={statusTone(event.severity)}>{event.severity}</StatusPill>
                </header>
                <p>{event.summary}</p>
                <small>{event.createdAt}</small>
              </button>
            ))
          )}
        </div>
      </section>
    </section>
  );
}

interface MetricSelection {
  label: string;
  value: string;
  detail: string;
}

function selection(label: string, value: string, detail: string): MetricSelection {
  return { label, value, detail };
}

function ProviderMetricRow({
  provider,
  onSelect,
}: {
  provider: ProviderRuntimeMetric;
  onSelect: () => void;
}) {
  return (
    <button type="button" className="metrics-row metrics-row--provider" onClick={onSelect}>
      <div>
        <strong>{provider.providerId}</strong>
        <span>
          {provider.requestCount} requests · {provider.errorCount} errors · {provider.retryCount} retries
        </span>
        <small>
          {provider.inputTokens + provider.outputTokens} tokens · {costLabel(provider.estimatedCostUsd)} ·{" "}
          {latencyLabel(provider.latencyMsP95)}
        </small>
      </div>
      <StatusPill tone={statusTone(provider.status)}>{provider.status}</StatusPill>
    </button>
  );
}

function MetricCard({
  icon,
  label,
  value,
  detail,
  onSelect,
}: {
  icon: React.ReactNode;
  label: string;
  value: string | number;
  detail: string;
  onSelect: () => void;
}) {
  return (
    <button type="button" className="metrics-card" aria-label={label} onClick={onSelect}>
      <span className="metrics-card__icon">{icon}</span>
      <span className="metric-tile__label">{label}</span>
      <strong>{value}</strong>
      <span className="metric-tile__detail">{detail}</span>
    </button>
  );
}

function MetricRow({
  item,
  onSelect,
}: {
  item: { label: string; value: string; unit: string; status: string };
  onSelect: () => void;
}) {
  return (
    <button type="button" className="metrics-row" onClick={onSelect}>
      <div>
        <strong>{item.label}</strong>
        <span>
          {item.value} {item.unit}
        </span>
      </div>
      <StatusPill tone={statusTone(item.status)}>{item.status}</StatusPill>
    </button>
  );
}

function statusTone(status: string): "good" | "warn" | "muted" {
  if (["good", "ready", "active", "info"].includes(status)) {
    return "good";
  }
  if (["warn", "missing", "failed"].includes(status)) {
    return "warn";
  }
  return "muted";
}

function formatBytes(value: number) {
  if (value < 1024) {
    return `${value} B`;
  }
  if (value < 1024 * 1024) {
    return `${Math.round(value / 1024)} KB`;
  }
  return `${(value / (1024 * 1024)).toFixed(1)} MB`;
}

function costLabel(value: number) {
  return value > 0 ? `$${value.toFixed(4)}` : "cost n/a";
}

function latencyLabel(value: number) {
  return value > 0 ? `p95 ${value} ms` : "latency n/a";
}
