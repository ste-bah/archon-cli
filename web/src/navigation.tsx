import {
  Activity,
  Brain,
  ChartNoAxesCombined,
  Database,
  FileSearch,
  Gauge,
  GitBranch,
  PackagePlus,
  Network,
  MessageSquare,
  Settings,
  type LucideIcon,
} from "lucide-react";

export interface NavItem {
  path: string;
  label: string;
  detail: string;
  icon: LucideIcon;
}

export const navItems: NavItem[] = [
  {
    path: "/",
    label: "Overview",
    detail: "Runtime status and system posture",
    icon: Activity,
  },
  {
    path: "/chat",
    label: "Chat",
    detail: "Dynamic chat with attachments",
    icon: MessageSquare,
  },
  {
    path: "/corpus",
    label: "Corpus",
    detail: "Docs, KBs, chunks, and source viewers",
    icon: FileSearch,
  },
  {
    path: "/ingest",
    label: "Ingest",
    detail: "Documents, images, videos, and KBs",
    icon: PackagePlus,
  },
  {
    path: "/memory",
    label: "Memory",
    detail: "Learning rows, memories, and proposals",
    icon: Brain,
  },
  {
    path: "/world",
    label: "World Model",
    detail: "Predictions, reasoning, candidates",
    icon: Database,
  },
  {
    path: "/jepa",
    label: "JEPA",
    detail: "Candidates, eval gates, Metal/CUDA parity",
    icon: Network,
  },
  {
    path: "/pipelines",
    label: "Pipelines",
    detail: "Stages, agents, artifacts, output",
    icon: GitBranch,
  },
  {
    path: "/metrics",
    label: "Metrics",
    detail: "Performance, costs, latency, health",
    icon: Gauge,
  },
  {
    path: "/settings",
    label: "Settings",
    detail: "Theme, policy posture, web config",
    icon: Settings,
  },
  {
    path: "/evidence",
    label: "Evidence",
    detail: "Graph fixture and relation exploration",
    icon: ChartNoAxesCombined,
  },
];
