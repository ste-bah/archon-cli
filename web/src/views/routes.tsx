import { lazy, Suspense } from "react";
import { Route, Routes } from "react-router-dom";
import { ChatPage } from "./ChatPage";
import { CognitivePage } from "./CognitivePage";
import { CorpusPage } from "./CorpusPage";
import { DashboardPage } from "./DashboardPage";
import { InspectionPage } from "./InspectionPage";
import { IngestPage } from "./IngestPage";
import { JepaPage } from "./JepaPage";
import { MemoryPage } from "./MemoryPage";
import { MetricsPage } from "./MetricsPage";
import { PipelinePage } from "./PipelinePage";
import { SettingsPage } from "./SettingsPage";
import { WorldPage } from "./WorldPage";
import type {
  ApiStatus,
  CognitiveWebSummary,
  CorpusSummary,
  EvidenceGraphSummary,
  EffectiveConfigSummary,
  EffectivePolicySummary,
  LearningSummary,
  MetricsSummary,
  PipelineSummary,
  SettingsSummary,
  WebUploadPolicy,
  WebIngestSummary,
  WorkflowWebSummary,
  WorldInspectionSummary,
} from "../api/generated/web";
import { WorkflowPage } from "./WorkflowPage";

const EvidenceGraphPage = lazy(() =>
  import("./EvidenceGraphPage").then((module) => ({
    default: module.EvidenceGraphPage,
  })),
);

interface WorkbenchRoutesProps {
  status?: ApiStatus;
  config?: EffectiveConfigSummary;
  policy?: EffectivePolicySummary;
  liveCount?: number;
  authRequired?: boolean;
  uploadsEnabled?: boolean;
  uploadPolicy?: WebUploadPolicy;
  corpus?: CorpusSummary;
  ingest?: WebIngestSummary;
  learning?: LearningSummary;
  cognitive?: CognitiveWebSummary;
  world?: WorldInspectionSummary;
  pipelines?: PipelineSummary;
  workflows?: WorkflowWebSummary;
  metrics?: MetricsSummary;
  evidence?: EvidenceGraphSummary;
  settings?: SettingsSummary;
  theme: "dark" | "light";
  onThemeToggle: () => void;
}

export function WorkbenchRoutes(props: WorkbenchRoutesProps) {
  return (
    <Routes>
      <Route path="/" element={<DashboardPage {...props} />} />
      <Route path="/chat" element={<ChatPage uploadPolicy={props.uploadPolicy} />} />
      <Route path="/corpus" element={<CorpusPage corpus={props.corpus} />} />
      <Route path="/ingest" element={<IngestPage ingest={props.ingest} />} />
      <Route
        path="/memory"
        element={<MemoryPage learning={props.learning} />}
      />
      <Route
        path="/cognitive"
        element={<CognitivePage cognitive={props.cognitive} />}
      />
      <Route
        path="/world"
        element={<WorldPage world={props.world} />}
      />
      <Route
        path="/jepa"
        element={<JepaPage world={props.world} />}
      />
      <Route
        path="/pipelines"
        element={<PipelinePage pipelines={props.pipelines} />}
      />
      <Route
        path="/workflows"
        element={<WorkflowPage workflows={props.workflows} />}
      />
      <Route
        path="/metrics"
        element={<MetricsPage metrics={props.metrics} liveCount={props.liveCount} />}
      />
      <Route
        path="/settings"
        element={
          <SettingsPage
            settings={props.settings}
            authRequired={props.authRequired}
            uploadsEnabled={props.uploadsEnabled}
            theme={props.theme}
            onThemeToggle={props.onThemeToggle}
          />
        }
      />
      <Route
        path="/evidence"
        element={
          <Suspense
            fallback={
              <InspectionPage
                eyebrow="Graph"
                title="Evidence graph"
                summary="Loading the graph renderer for the evidence relationship view."
                facts={[fact("Renderer", "loading")]}
                rows={[]}
              />
            }
          >
            <EvidenceGraphPage graph={props.evidence} />
          </Suspense>
        }
      />
    </Routes>
  );
}

function fact(label: string, value?: string | number, tone?: "good" | "warn" | "muted") {
  return { label, value: value === undefined ? "loading" : String(value), tone };
}
