import { BookOpen, Database, FileUp, RefreshCw, Video } from "lucide-react";
import { useMemo, useState } from "react";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { apiClient } from "../api/client";
import { StatusPill } from "../components/StatusPill";
import type {
  WebDocStoreItem,
  WebIngestJob,
  WebIngestSummary,
  WebKnowledgeBaseItem,
  WebVideoStoreItem,
} from "../api/generated/web";
import "./IngestPage.css";

interface IngestPageProps {
  ingest?: WebIngestSummary;
}

type ViewerTab = "documents" | "videos" | "kbs";
type ViewerItem = WebDocStoreItem | WebVideoStoreItem | WebKnowledgeBaseItem;

export function IngestPage({ ingest }: IngestPageProps) {
  const queryClient = useQueryClient();
  const [target, setTarget] = useState("docs");
  const [source, setSource] = useState("");
  const [videoSource, setVideoSource] = useState("");
  const [frames, setFrames] = useState("hybrid");
  const [asr, setAsr] = useState("whisper-cpp");
  const [vlm, setVlm] = useState(true);
  const [kbName, setKbName] = useState("");
  const [kbScope, setKbScope] = useState("project");
  const [tab, setTab] = useState<ViewerTab>("documents");
  const [selectedKey, setSelectedKey] = useState("");
  const ingestMutation = useMutation({
    mutationFn: apiClient.startIngest,
    onSettled: () => queryClient.invalidateQueries({ queryKey: ["ingest"] }),
  });
  const kbMutation = useMutation({
    mutationFn: apiClient.createKnowledgeBase,
    onSettled: () => queryClient.invalidateQueries({ queryKey: ["ingest"] }),
  });
  const items = useMemo(() => viewerItems(ingest, tab), [ingest, tab]);
  const selected = items.find((item) => itemKey(item) === selectedKey) ?? items[0];

  const disabled = !ingest?.allowed || ingestMutation.isPending || kbMutation.isPending;

  return (
    <section className="ingest-layout">
      <div className="panel ingest-hero">
        <div className="panel-heading">
          <div>
            <span className="eyebrow">Doc store</span>
            <h3>Ingest control</h3>
          </div>
          <StatusPill tone={ingest?.allowed ? "good" : "warn"}>
            {ingest?.allowed ? "enabled" : "policy blocked"}
          </StatusPill>
        </div>
        <div className="ingest-metrics">
          <Metric icon={<FileUp size={18} />} label="documents" value={ingest?.documents.length ?? 0} />
          <Metric icon={<Video size={18} />} label="videos" value={ingest?.videos.length ?? 0} />
          <Metric icon={<BookOpen size={18} />} label="KBs" value={ingest?.knowledgeBases.length ?? 0} />
          <Metric icon={<Database size={18} />} label="chunks" value={ingest?.kbStats.chunks ?? 0} />
        </div>
        <p className="summary">{ingest?.policyReason ?? "Loading ingest policy."}</p>
      </div>

      <div className="ingest-grid">
        <form
          className="panel ingest-form"
          onSubmit={(event) => {
            event.preventDefault();
            ingestMutation.mutate({
              target,
              source,
              frames: null,
              asr: null,
              transcript: null,
              vlm: false,
              metadataOnly: false,
              confirmed: true,
            });
          }}
        >
          <FormHeading icon={<FileUp size={18} />} title="Documents and KB sources" />
          <label>
            <span>Target</span>
            <select value={target} onChange={(event) => setTarget(event.target.value)}>
              <option value="docs">Document store</option>
              <option value="kb">Knowledge base</option>
              <option value="kb_process">Process KB graph</option>
            </select>
          </label>
          {target !== "kb_process" && (
            <label>
              <span>Path or URL</span>
              <input
                value={source}
                onChange={(event) => setSource(event.target.value)}
                placeholder="/path/file.pdf or https://..."
              />
            </label>
          )}
          <CommandPreview command={target === "kb_process" ? "archon kb process --claims --entities --relations --contradictions" : `archon ${target === "kb" ? "kb" : "docs"} ingest ${source || "<source>"}`} />
          <button type="submit" disabled={disabled}>
            <RefreshCw size={16} /> Run
          </button>
        </form>

        <form
          className="panel ingest-form"
          onSubmit={(event) => {
            event.preventDefault();
            ingestMutation.mutate({
              target: "video",
              source: videoSource,
              frames,
              asr,
              transcript: null,
              vlm,
              metadataOnly: false,
              confirmed: true,
            });
          }}
        >
          <FormHeading icon={<Video size={18} />} title="Video evidence" />
          <label>
            <span>Video path or URL</span>
            <input
              value={videoSource}
              onChange={(event) => setVideoSource(event.target.value)}
              placeholder="https://youtu.be/... or /path/video.mp4"
            />
          </label>
          <div className="ingest-two">
            <label>
              <span>Frames</span>
              <select value={frames} onChange={(event) => setFrames(event.target.value)}>
                <option value="hybrid">hybrid</option>
                <option value="scene">scene</option>
                <option value="interval">interval</option>
                <option value="none">none</option>
              </select>
            </label>
            <label>
              <span>ASR</span>
              <select value={asr} onChange={(event) => setAsr(event.target.value)}>
                <option value="whisper-cpp">whisper-cpp</option>
                <option value="faster-whisper">faster-whisper</option>
                <option value="whisper-rs">whisper-rs</option>
                <option value="disabled">disabled</option>
              </select>
            </label>
          </div>
          <label className="ingest-check">
            <input type="checkbox" checked={vlm} onChange={(event) => setVlm(event.target.checked)} />
            <span>Frame VLM</span>
          </label>
          <CommandPreview command={`archon video ingest ${videoSource || "<source>"} --frames ${frames} --asr ${asr} ${vlm ? "--vlm " : ""}--yes`} />
          <button type="submit" disabled={disabled}>
            <RefreshCw size={16} /> Run
          </button>
        </form>

        <form
          className="panel ingest-form"
          onSubmit={(event) => {
            event.preventDefault();
            kbMutation.mutate({
              name: kbName,
              scope: kbScope,
              description: "Created from the Archon web workbench.",
              confirmed: true,
            });
          }}
        >
          <FormHeading icon={<BookOpen size={18} />} title="Create KB" />
          <label>
            <span>Name</span>
            <input value={kbName} onChange={(event) => setKbName(event.target.value)} placeholder="project evidence" />
          </label>
          <label>
            <span>Scope</span>
            <select value={kbScope} onChange={(event) => setKbScope(event.target.value)}>
              <option value="project">project</option>
              <option value="home">home</option>
            </select>
          </label>
          <button type="submit" disabled={disabled}>
            <BookOpen size={16} /> Create
          </button>
        </form>
      </div>

      <div className="ingest-bottom">
        <Viewer
          tab={tab}
          setTab={(next) => {
            setTab(next);
            setSelectedKey("");
          }}
          items={items}
          selected={selected}
          onSelect={(item) => setSelectedKey(itemKey(item))}
        />
        <Jobs jobs={ingest?.jobs ?? []} warnings={ingest?.warnings ?? []} />
      </div>
    </section>
  );
}

function Metric({ icon, label, value }: { icon: React.ReactNode; label: string; value: number }) {
  return (
    <div className="ingest-metric">
      {icon}
      <span>{label}</span>
      <strong>{value}</strong>
    </div>
  );
}

function FormHeading({ icon, title }: { icon: React.ReactNode; title: string }) {
  return (
    <div className="ingest-form-heading">
      {icon}
      <strong>{title}</strong>
    </div>
  );
}

function CommandPreview({ command }: { command: string }) {
  return <code className="ingest-command">{command}</code>;
}

function Viewer({
  tab,
  setTab,
  items,
  selected,
  onSelect,
}: {
  tab: ViewerTab;
  setTab: (tab: ViewerTab) => void;
  items: ViewerItem[];
  selected?: ViewerItem;
  onSelect: (item: ViewerItem) => void;
}) {
  return (
    <div className="panel ingest-viewer">
      <div className="panel-heading">
        <div>
          <span className="eyebrow">Viewer</span>
          <h3>Store items</h3>
        </div>
        <StatusPill>{items.length} items</StatusPill>
      </div>
      <div className="ingest-tabs">
        {(["documents", "videos", "kbs"] as const).map((item) => (
          <button key={item} type="button" className={tab === item ? "active" : ""} onClick={() => setTab(item)}>
            {item}
          </button>
        ))}
      </div>
      <div className="ingest-viewer-grid">
        <div className="ingest-item-list">
          {items.map((item) => (
            <button key={itemKey(item)} type="button" onClick={() => onSelect(item)}>
              <strong>{itemTitle(item)}</strong>
              <small>{itemSubtitle(item)}</small>
            </button>
          ))}
        </div>
        <pre className="ingest-detail">{selected ? JSON.stringify(selected, null, 2) : "No items yet."}</pre>
      </div>
    </div>
  );
}

function Jobs({ jobs, warnings }: { jobs: WebIngestJob[]; warnings: string[] }) {
  return (
    <div className="panel ingest-jobs">
      <div className="panel-heading">
        <div>
          <span className="eyebrow">Jobs</span>
          <h3>Recent ingest runs</h3>
        </div>
        <StatusPill>{jobs.length} jobs</StatusPill>
      </div>
      {warnings.map((warning) => (
        <p key={warning} className="ingest-warning">{warning}</p>
      ))}
      {jobs.length === 0 ? (
        <p className="summary">No web ingest jobs yet.</p>
      ) : (
        jobs.map((job) => (
          <div key={job.jobId} className="ingest-job">
            <div>
              <strong>{job.label || job.target}</strong>
              <small>{job.command}</small>
            </div>
            <StatusPill tone={job.status === "failed" ? "warn" : job.status === "completed" ? "good" : "muted"}>{job.status}</StatusPill>
            {(job.stdoutTail || job.stderrTail) && <pre>{job.stdoutTail || job.stderrTail}</pre>}
          </div>
        ))
      )}
    </div>
  );
}

function viewerItems(summary: WebIngestSummary | undefined, tab: ViewerTab): ViewerItem[] {
  if (!summary) return [];
  if (tab === "videos") return summary.videos;
  if (tab === "kbs") return summary.knowledgeBases;
  return summary.documents;
}

function itemKey(item: ViewerItem) {
  if (isVideoItem(item)) return item.videoId;
  if (isDocItem(item)) return item.documentId;
  return item.path;
}

function itemTitle(item: ViewerItem) {
  if (isDocItem(item)) return item.sourcePath.split("/").pop() || item.documentId;
  if (isVideoItem(item)) return item.title || item.videoId;
  return item.name;
}

function itemSubtitle(item: ViewerItem) {
  if (isDocItem(item)) return `${item.status} · ${item.chunks} chunks`;
  if (isVideoItem(item)) return `${item.status} · ${item.frames} frames`;
  return `${item.scope} · ${item.files} files`;
}

function isDocItem(item: ViewerItem): item is WebDocStoreItem {
  return "sourcePath" in item;
}

function isVideoItem(item: ViewerItem): item is WebVideoStoreItem {
  return "videoId" in item;
}
