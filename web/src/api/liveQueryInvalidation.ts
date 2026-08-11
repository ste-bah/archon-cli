//! The consumption half of the live event stream: which surfaces a live event
//! makes stale, and how often that question is allowed to cost a request.
//!
//! The stream exists so the workbench does not have to poll. The mapping below
//! is therefore deliberately narrow: an event only appears here when the
//! endpoint behind the query reads state the event's recording site is known to
//! have changed. Anything speculative would just be a poll with extra steps.

import { useQueryClient } from "@tanstack/react-query";
import { useEffect, useRef } from "react";
import type { WebLiveEvent } from "./generated/web";
import type { LiveFeed } from "./useLiveEvents";

/** Query keys in `App.tsx` that a live event can make stale. */
export type LiveQueryKey = "cognitive" | "corpus" | "evidence" | "ingest" | "learning" | "metrics";

/**
 * Every `eventType` the server records, and the queries it invalidates.
 *
 * The list is exhaustive over `WebLiveManager::record` call sites rather than
 * over anything the UI wishes existed, and the empty entries are kept so a new
 * recording site shows up as a missing key rather than as silence. Sites, in
 * `crates/archon-sdk/src/web`:
 *
 * - `ingest_jobs.rs` — `web.ingest.started` / `web.ingest.finished`
 * - `docs_actions.rs` — `web.docs.deleted` / `web.index.control`
 * - `chat.rs` — `web.chat.submitted` / `web.chat.failed` / `web.chat.completed`
 * - `uploads_receive.rs` — `web.upload.stored`
 * - `server.rs` — `web.runtime.started`
 */
export const LIVE_EVENT_QUERY_KEYS: Readonly<Record<string, readonly LiveQueryKey[]>> = {
  // A job row is pushed onto the in-memory job store before the child process
  // is spawned, so `/api/ingest/summary` has a new `running` row to report.
  "web.ingest.started": ["ingest"],
  // The job's exit status, stdout tail and finish time land on the same row,
  // and the command it ran has by then written into the docs store and the
  // corpus roots (`.archon/docs`, `.archon/kb`) that `/api/corpus/summary` and
  // `/api/evidence/graph` walk.
  "web.ingest.finished": ["ingest", "corpus", "evidence"],
  // `archon_docs::delete::delete_document` removes the document, its chunks,
  // pages and artifacts — the document list, the corpus source list and the
  // evidence graph's per-source stats all move.
  "web.docs.deleted": ["ingest", "corpus", "evidence"],
  // Pause/resume/cancel/retry on the semantic index queue. Queue and index-job
  // counters move; no file under a corpus root does.
  "web.index.control": ["ingest"],
  // Recorded before the turn runs, when nothing durable has been written yet.
  // `web.chat.completed`/`failed` follow within the same request, so acting
  // here would only buy a refetch of state that has not changed.
  "web.chat.submitted": [],
  // The turn ran inside a real session: `process_message_steps` classifies and
  // stores a cognitive situation, the session activity and reasoning-quality
  // ledgers grow, and the provider runtime event log picks up the call. Those
  // are exactly the files `/api/cognitive/summary`, `/api/learning/summary` and
  // `/api/metrics/summary` read.
  "web.chat.completed": ["cognitive", "learning", "metrics"],
  // A failed turn still burned a provider call and still recorded its
  // situation, so the same three surfaces move.
  "web.chat.failed": ["cognitive", "learning", "metrics"],
  // Staging-only: the bytes land under `~/.archon/web/uploads`, which no
  // summary endpoint reads. The docs-store side of an attachment is reported by
  // `web.chat.completed` instead.
  "web.upload.stored": [],
  // Recorded during bind, before this client existed. The first fetch of every
  // query already reflects it.
  "web.runtime.started": [],
};

/**
 * Coalescing window. A run emits events in bursts, and invalidating per event
 * would turn one busy second into a request per surface per event.
 *
 * The window opens on the first event that maps to anything and flushes the
 * union of everything that arrived inside it, so a burst of any size costs at
 * most one refetch per distinct surface. 250 ms is short enough to stay under
 * what reads as "instant" and comfortably shorter than the server's own 1 s
 * stream poll, so a single frame never gets split across two flushes.
 */
export const LIVE_INVALIDATION_WINDOW_MS = 250;

export function queryKeysForEvent(eventType: string): readonly LiveQueryKey[] {
  return LIVE_EVENT_QUERY_KEYS[eventType] ?? [];
}

/** Every surface the stream can touch — what a cursor gap has to assume moved. */
export const ALL_LIVE_QUERY_KEYS: readonly LiveQueryKey[] = [
  ...new Set(Object.values(LIVE_EVENT_QUERY_KEYS).flat()),
].sort();

/**
 * Query keys made stale by the events newer than `processedCursor` that also
 * arrived at or after `backlogCursor`.
 *
 * Split out from the hook so the mapping is testable without a React tree, and
 * so the cursor arithmetic has one home.
 */
export function collectQueryKeys(
  events: readonly WebLiveEvent[],
  backlogCursor: number | undefined,
  processedCursor: number,
): LiveQueryKey[] {
  if (backlogCursor === undefined) {
    return [];
  }
  const keys = new Set<LiveQueryKey>();
  for (const event of events) {
    if (event.cursor < backlogCursor || event.cursor <= processedCursor) {
      continue;
    }
    for (const key of queryKeysForEvent(event.eventType)) {
      keys.add(key);
    }
  }
  return [...keys];
}

/** Highest cursor in `events`, or `fallback` when the feed is empty. */
export function highestCursor(events: readonly WebLiveEvent[], fallback: number): number {
  return events.reduce((highest, event) => Math.max(highest, event.cursor), fallback);
}

/**
 * Drive query invalidation from the live feed.
 *
 * Mounted once, next to the queries it invalidates, so every surface reacts to
 * the stream whether or not its tab is the one on screen — the queries live in
 * `App.tsx` and stay active for the life of the page.
 */
export function useLiveQueryInvalidation(
  feed: LiveFeed,
  windowMs: number = LIVE_INVALIDATION_WINDOW_MS,
): void {
  const queryClient = useQueryClient();
  const processedCursor = useRef(0);
  const seenRecoveries = useRef(feed.recoveries);
  const pending = useRef(new Set<LiveQueryKey>());
  const timer = useRef<number | undefined>(undefined);

  const { events, recoveries, backlogCursor } = feed;

  useEffect(() => {
    const keys = new Set<LiveQueryKey>();

    if (recoveries !== seenRecoveries.current) {
      // The server buffer compacted past where we were reading, so the events
      // that would have told us what moved are gone. The honest response to a
      // known gap is to assume every mapped surface moved.
      seenRecoveries.current = recoveries;
      processedCursor.current = 0;
      for (const key of ALL_LIVE_QUERY_KEYS) {
        keys.add(key);
      }
    }

    for (const key of collectQueryKeys(events, backlogCursor, processedCursor.current)) {
      keys.add(key);
    }
    if (backlogCursor !== undefined) {
      processedCursor.current = highestCursor(events, processedCursor.current);
    }

    if (keys.size === 0) {
      return;
    }
    for (const key of keys) {
      pending.current.add(key);
    }
    if (timer.current !== undefined) {
      // A window is already open; this burst joins it rather than opening a
      // second one.
      return;
    }
    timer.current = window.setTimeout(() => {
      timer.current = undefined;
      const flushing = [...pending.current];
      pending.current.clear();
      for (const key of flushing) {
        void queryClient.invalidateQueries({ queryKey: [key] });
      }
    }, windowMs);
  }, [events, recoveries, backlogCursor, queryClient, windowMs]);

  useEffect(
    () => () => {
      if (timer.current !== undefined) {
        window.clearTimeout(timer.current);
        timer.current = undefined;
      }
    },
    [],
  );
}
