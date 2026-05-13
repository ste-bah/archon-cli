import { useQuery } from "@tanstack/react-query";
import { useEffect, useState } from "react";
import { HashRouter } from "react-router-dom";
import { apiClient } from "./api/client";
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
  const live = useQuery({ queryKey: ["live"], queryFn: apiClient.liveSnapshot });
  const auth = useQuery({ queryKey: ["auth"], queryFn: apiClient.authSession });
  const uploads = useQuery({ queryKey: ["uploads"], queryFn: apiClient.uploadPolicy });
  const corpus = useQuery({ queryKey: ["corpus"], queryFn: apiClient.corpusSummary });
  const learning = useQuery({ queryKey: ["learning"], queryFn: apiClient.learningSummary });
  const world = useQuery({ queryKey: ["world"], queryFn: apiClient.worldSummary });
  const pipelines = useQuery({ queryKey: ["pipelines"], queryFn: apiClient.pipelineSummary });
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
          liveCount={live.data?.events.length}
          authRequired={auth.data?.authRequired}
          uploadsEnabled={uploads.data?.enabled}
          uploadPolicy={uploads.data}
          corpus={corpus.data}
          learning={learning.data}
          world={world.data}
          pipelines={pipelines.data}
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
