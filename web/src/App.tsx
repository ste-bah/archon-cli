import { useQuery } from "@tanstack/react-query";
import { useEffect, useState } from "react";
import { HashRouter } from "react-router";
import { apiClient } from "./api/client";
import { useLiveQueryInvalidation } from "./api/liveQueryInvalidation";
import { useLiveEvents } from "./api/useLiveEvents";
import { AppShell } from "./components/AppShell";
import { WorkbenchRoutes } from "./views/routes";

type ThemeMode = "dark" | "light";

export function App() {
  const [theme, setTheme] = useState<ThemeMode>(() =>
    window.localStorage.getItem("archon.theme") === "light" ? "light" : "dark",
  );
  const status = useQuery({ queryKey: ["status"], queryFn: apiClient.status });
  const config = useQuery({ queryKey: ["config"], queryFn: apiClient.config });
  const policy = useQuery({ queryKey: ["policy"], queryFn: apiClient.policy });
  const live = useLiveEvents();
  // The rest of these queries are fetch-once under the client's 10s staleTime.
  // Rather than giving each one a timer, the live stream tells them when the
  // state behind them actually moved; see `LIVE_EVENT_QUERY_KEYS` for the
  // event-to-surface mapping and for the surfaces that have no event at all.
  useLiveQueryInvalidation(live);
  const agents = useQuery({
    queryKey: ["agents"],
    queryFn: apiClient.agentsLive,
    // Current state, not an append-only log, so it is polled rather than
    // streamed. Same conditional-interval shape as `ingest`: fast while
    // something is running, slow when idle — but never off, because a new
    // agent appearing is exactly what this view exists to show.
    refetchInterval: (query) => (query.state.data?.agents.length ? 5000 : 10000),
  });
  const auth = useQuery({ queryKey: ["auth"], queryFn: apiClient.authSession });
  const uploads = useQuery({ queryKey: ["uploads"], queryFn: apiClient.uploadPolicy });
  const corpus = useQuery({ queryKey: ["corpus"], queryFn: apiClient.corpusSummary });
  const ingest = useQuery({
    queryKey: ["ingest"],
    queryFn: apiClient.ingestSummary,
    refetchInterval: (query) =>
      query.state.data?.jobs.some((job) => job.status === "running")
      || query.state.data?.indexJobs.some((job) => job.status === "running")
        ? 2500
        : false,
  });
  const learning = useQuery({ queryKey: ["learning"], queryFn: apiClient.learningSummary });
  const cognitive = useQuery({ queryKey: ["cognitive"], queryFn: apiClient.cognitiveSummary });
  const world = useQuery({ queryKey: ["world"], queryFn: apiClient.worldSummary });
  const pipelines = useQuery({ queryKey: ["pipelines"], queryFn: apiClient.pipelineSummary });
  const workflows = useQuery({ queryKey: ["workflows"], queryFn: apiClient.workflowSummary });
  const metrics = useQuery({ queryKey: ["metrics"], queryFn: apiClient.metricsSummary });
  const evidence = useQuery({ queryKey: ["evidence"], queryFn: apiClient.evidenceGraph });
  const settings = useQuery({ queryKey: ["settings"], queryFn: apiClient.settingsSummary });

  useEffect(() => {
    document.documentElement.dataset.theme = theme;
    window.localStorage.setItem("archon.theme", theme);
  }, [theme]);

  return (
    <HashRouter>
      <AppShell
        status={status.data}
        theme={theme}
        onThemeToggle={() => setTheme(theme === "dark" ? "light" : "dark")}
      >
        {(status.isError || config.isError || policy.isError) && (
          <div className="error-banner" role="alert">
            Web API connection failed. Check that `archon web` is still running.
          </div>
        )}
        <WorkbenchRoutes
          status={status.data}
          config={config.data}
          policy={policy.data}
          liveCount={live.events.length}
          liveEvents={live.events}
          liveStatus={live.status}
          agents={agents.data}
          authRequired={auth.data?.authRequired}
          uploadsEnabled={uploads.data?.enabled}
          uploadPolicy={uploads.data}
          corpus={corpus.data}
          ingest={ingest.data}
          learning={learning.data}
          cognitive={cognitive.data}
          world={world.data}
          pipelines={pipelines.data}
          workflows={workflows.data}
          metrics={metrics.data}
          evidence={evidence.data}
          settings={settings.data}
          theme={theme}
          onThemeToggle={() => setTheme(theme === "dark" ? "light" : "dark")}
        />
      </AppShell>
    </HashRouter>
  );
}
