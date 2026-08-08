import { useState } from "react";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Ban, Pause, Play, RotateCcw } from "lucide-react";
import { apiClient } from "../../api/client";
import type { WebIndexJobItem } from "../../api/generated/web";

interface IndexControlsProps {
  jobs: WebIndexJobItem[];
  failed: number;
  enabled: boolean;
}

/**
 * Buttons for the six index verbs the page could already count but not run.
 *
 * `index-status`, `index-pause`, `index-resume`, `index-cancel` and
 * `index-retry-failed` are five subcommands over one piece of stateful
 * machinery, and the state was already on screen — pending, leased, indexed,
 * failed — with no way to act on it. Reading a number and then going to a
 * terminal to type the verb that changes it is the exact split this closes.
 */
export function IndexControls({ jobs, failed, enabled }: IndexControlsProps) {
  const queryClient = useQueryClient();
  const [note, setNote] = useState("");
  const control = useMutation({
    mutationFn: apiClient.indexControl,
    onSuccess: (result) =>
      setNote(result.accepted ? result.detail : result.policyReason),
    onError: (error: unknown) => setNote(String(error)),
    onSettled: () => queryClient.invalidateQueries({ queryKey: ["ingest"] }),
  });

  // Only a job that is actually running can be paused or cancelled, and only a
  // paused one can resume. Offering every verb on every row invites a click
  // that the backend will refuse for reasons the UI already knew.
  const running = jobs.filter((job) => job.status === "running");
  const paused = jobs.filter((job) => job.status === "paused");
  const busy = control.isPending || !enabled;

  return (
    <div className="ingest-index-controls">
      {running.map((job) => (
        <div key={job.jobId} className="ingest-index-control-row">
          <code>{job.jobId.slice(0, 8)}</code>
          <span>running · {job.indexed} indexed</span>
          <button
            type="button"
            disabled={busy}
            onClick={() => control.mutate({ action: "pause", jobId: job.jobId, limit: null })}
          >
            <Pause size={13} aria-hidden="true" /> Pause
          </button>
          <button
            type="button"
            className="danger"
            disabled={busy}
            onClick={() => control.mutate({ action: "cancel", jobId: job.jobId, limit: null })}
          >
            <Ban size={13} aria-hidden="true" /> Cancel
          </button>
        </div>
      ))}
      {paused.map((job) => (
        <div key={job.jobId} className="ingest-index-control-row">
          <code>{job.jobId.slice(0, 8)}</code>
          <span>paused · {job.indexed} indexed</span>
          <button
            type="button"
            disabled={busy}
            onClick={() => control.mutate({ action: "resume", jobId: job.jobId, limit: null })}
          >
            <Play size={13} aria-hidden="true" /> Resume
          </button>
        </div>
      ))}
      <div className="ingest-index-control-row">
        <span>{failed} failed chunk(s)</span>
        <button
          type="button"
          disabled={busy || failed === 0}
          onClick={() => control.mutate({ action: "retryFailed", jobId: null, limit: null })}
        >
          <RotateCcw size={13} aria-hidden="true" /> Retry failed
        </button>
      </div>
      {note ? <p className="ingest-index-note">{note}</p> : null}
    </div>
  );
}
