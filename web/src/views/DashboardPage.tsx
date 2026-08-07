import { MetricTile } from "../components/MetricTile";
import { StatusPill } from "../components/StatusPill";
import type {
  ApiStatus,
  EffectiveConfigSummary,
  EffectivePolicySummary,
  WebAgentActivitySnapshot,
  WebLiveEvent,
} from "../api/generated/web";
import type { LiveStatus } from "../api/useLiveEvents";

interface DashboardPageProps {
  status?: ApiStatus;
  config?: EffectiveConfigSummary;
  policy?: EffectivePolicySummary;
  liveCount?: number;
  liveEvents?: WebLiveEvent[];
  liveStatus?: LiveStatus;
  agents?: WebAgentActivitySnapshot;
  authRequired?: boolean;
  uploadsEnabled?: boolean;
}

export function DashboardPage({
  status,
  config,
  policy,
  liveCount,
  liveEvents,
  liveStatus,
  agents,
  authRequired,
  uploadsEnabled,
}: DashboardPageProps) {
  const features = status?.features;
  return (
    <div className="page-grid">
      <section className="panel panel--wide">
        <div className="panel-heading">
          <div>
            <span className="eyebrow">Workbench foundation</span>
            <h3>Runtime posture</h3>
          </div>
          <StatusPill tone={status?.web.devMode ? "warn" : "good"}>
            {status?.web.assetMode ?? "loading"}
          </StatusPill>
        </div>
        <div className="metric-grid">
          <MetricTile
            label="Version"
            value={status?.version ?? "loading"}
            detail="archon-sdk web surface"
          />
          <MetricTile
            label="Bind"
            value={`${status?.web.bindAddress ?? "-"}:${status?.web.port ?? "-"}`}
            detail={config?.web.nonLoopbackBind ? "network exposed" : "local only"}
          />
          <MetricTile
            label="Policy"
            value={policy?.web.allowMutatingActions ? "actions enabled" : "inspect only"}
            detail={policy?.actionGate ?? "loading policy composition"}
          />
          <MetricTile
            label="Live events"
            value={`${liveCount ?? 0}`}
            detail="streamed from the event log"
          />
          <MetricTile
            label="Uploads"
            value={uploadsEnabled ? "enabled" : "disabled"}
            detail="policy-gated attachment lane"
          />
        </div>
      </section>

      <section className="panel panel--wide">
        <div className="panel-heading">
          <div>
            <span className="eyebrow">Live</span>
            <h3>Event feed</h3>
          </div>
          <StatusPill tone={liveStatusTone(liveStatus)}>{liveStatus ?? "connecting"}</StatusPill>
        </div>
        <div className="store-list">
          {(liveEvents ?? []).length === 0 && (
            <div className="store-row">
              <strong>no events yet</strong>
              <span>Events appear here as the session records them.</span>
            </div>
          )}
          {(liveEvents ?? []).slice(0, 25).map((event) => (
            <div key={event.cursor} className="store-row">
              <strong>{event.eventType}</strong>
              <span>{event.summary}</span>
            </div>
          ))}
        </div>
      </section>

      <section className="panel panel--wide">
        <div className="panel-heading">
          <div>
            <span className="eyebrow">Agents</span>
            <h3>Live agent activity</h3>
          </div>
          <StatusPill tone={(agents?.agents.length ?? 0) > 0 ? "good" : "muted"}>
            {agents ? `${agents.agents.length} running` : "loading"}
          </StatusPill>
        </div>
        <div className="store-list">
          {agents && agents.agents.length === 0 && (
            <div className="store-row">
              <strong>no agents running</strong>
              <span>
                {agents.attached
                  ? "Nothing is executing in this session right now."
                  : "Run the dashboard with /web inside the TUI to see live agents."}
              </span>
            </div>
          )}
          {agents?.agents.map((agent) => (
            <div key={`${agent.kind}-${agent.id}`} className="store-row">
              <strong>
                {agent.kind} · {agent.status}
              </strong>
              <span>
                {agent.label} — {formatElapsed(agent.elapsedMs)}
              </span>
            </div>
          ))}
        </div>
      </section>

      <section className="panel">
        <div className="panel-heading">
          <h3>Feature lanes</h3>
        </div>
        <div className="feature-list">
          {features &&
            Object.entries(features).map(([name, enabled]) => (
              <div key={name} className="feature-row">
                <span>{name}</span>
                <StatusPill tone={enabled ? "good" : "muted"}>
                  {enabled ? "visible" : "hidden"}
                </StatusPill>
              </div>
            ))}
          <div className="feature-row">
            <span>auth</span>
            <StatusPill tone={authRequired ? "warn" : "muted"}>
              {authRequired ? "required" : "loopback"}
            </StatusPill>
          </div>
        </div>
      </section>

      <section className="panel">
        <div className="panel-heading">
          <h3>Store adapters</h3>
        </div>
        <div className="store-list">
          {status?.stores.map((store) => (
            <div key={store.name} className="store-row">
              <strong>{store.name}</strong>
              <span>{store.detail}</span>
            </div>
          ))}
        </div>
      </section>
    </div>
  );
}

function liveStatusTone(status?: LiveStatus): "good" | "warn" | "muted" {
  if (status === "live") {
    return "good";
  }
  return status === "offline" ? "warn" : "muted";
}

function formatElapsed(elapsedMs: number): string {
  const seconds = Math.floor(elapsedMs / 1000);
  if (seconds < 60) {
    return `${seconds}s`;
  }
  const minutes = Math.floor(seconds / 60);
  return minutes < 60 ? `${minutes}m ${seconds % 60}s` : `${Math.floor(minutes / 60)}h ${minutes % 60}m`;
}
