import { Moon, Sun } from "lucide-react";
import { NavLink } from "react-router";
import { navItems } from "../navigation";
import { StatusPill } from "./StatusPill";
import type { ApiStatus } from "../api/generated/web";

interface AppShellProps {
  status?: ApiStatus;
  theme: "dark" | "light";
  onThemeToggle: () => void;
  children: React.ReactNode;
}

export function AppShell({ status, theme, onThemeToggle, children }: AppShellProps) {
  const ThemeIcon = theme === "dark" ? Sun : Moon;
  // Attached to a running session the TUI owns the conversation, so the server
  // builds no chat backend and reports the lane as unavailable. Hide the tab
  // rather than offer a surface every submit would refuse.
  // The terminal route is added to the router only when policy and the bind
  // address both allow it, so when the feature is off the path genuinely does
  // not exist and every connection would 404. Hide it for the same reason as
  // chat: do not offer a lane the server has refused to open.
  const items = navItems.filter(
    (item) =>
      (item.path !== "/chat" || status?.features.chat !== false) &&
      (item.path !== "/terminal" || status?.features.terminal === true),
  );
  return (
    <div className="workbench-shell">
      <aside className="sidebar" aria-label="Archon workbench sections">
        <div className="brand">
          <div className="brand-mark" aria-hidden="true">A</div>
          <div>
            <h1>Archon</h1>
            <p>Agentic workbench</p>
          </div>
        </div>
        <nav className="nav-list">
          {items.map((item) => (
            <NavLink
              key={item.path}
              to={item.path}
              end={item.path === "/"}
              className={({ isActive }) =>
                isActive ? "nav-item nav-item--active" : "nav-item"
              }
            >
              <item.icon size={18} aria-hidden="true" />
              <span>
                <strong>{item.label}</strong>
                <small>{item.detail}</small>
              </span>
            </NavLink>
          ))}
        </nav>
      </aside>
      <main className="main-surface">
        <header className="topbar">
          <div>
            <span className="eyebrow">Local control room</span>
            <h2>System inspection</h2>
          </div>
          <div className="topbar__status">
            <button
              className="icon-button"
              type="button"
              onClick={onThemeToggle}
              title={`Switch to ${theme === "dark" ? "light" : "dark"} theme`}
              aria-label={`Switch to ${theme === "dark" ? "light" : "dark"} theme`}
            >
              <ThemeIcon size={16} aria-hidden="true" />
            </button>
            <StatusPill tone={status?.status === "ok" ? "good" : "warn"}>
              {status?.status ?? "loading"}
            </StatusPill>
            <StatusPill tone={status?.web.authRequired ? "warn" : "muted"}>
              {status?.web.authRequired ? "auth required" : "loopback"}
            </StatusPill>
          </div>
        </header>
        {children}
      </main>
    </div>
  );
}
