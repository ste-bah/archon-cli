import { MetricTile } from "../components/MetricTile";
import { StatusPill } from "../components/StatusPill";
import type {
  ApiStatus,
  EffectiveConfigSummary,
  EffectivePolicySummary,
} from "../api/generated/web";

interface DashboardPageProps {
  status?: ApiStatus;
  config?: EffectiveConfigSummary;
  policy?: EffectivePolicySummary;
  liveCount?: number;
  authRequired?: boolean;
  uploadsEnabled?: boolean;
}

export function DashboardPage({
  status,
  config,
  policy,
  liveCount,
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
            detail="bounded snapshot buffer"
          />
          <MetricTile
            label="Uploads"
            value={uploadsEnabled ? "enabled" : "disabled"}
            detail="policy-gated attachment lane"
          />
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
