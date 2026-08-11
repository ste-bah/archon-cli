import { useEffect, useRef, useState } from "react";
import { apiClient, isCursorExpired } from "./client";
import type { WebLiveEvent } from "./generated/web";

/** Most recent events kept in memory; the server buffer is bounded too. */
const FEED_LIMIT = 200;
/**
 * A fetch-based SSE reader has no automatic reconnect, so this is hand-rolled.
 * Bounded: a server that is gone stays gone, and an unbounded retry loop on a
 * dead backend is a request storm with extra steps.
 */
const MAX_RECONNECTS = 5;
const RECONNECT_BASE_MS = 1000;

export type LiveStatus = "connecting" | "live" | "offline";

export interface LiveFeed {
  events: WebLiveEvent[];
  status: LiveStatus;
  /** Times the cursor expired and local state was dropped and refetched. */
  recoveries: number;
  /**
   * `nextCursor` of the backlog snapshot, i.e. the first cursor the server had
   * not issued yet when this client started listening.
   *
   * Everything below it is history that whatever fetched a surface has already
   * seen; everything at or above it happened while we were watching. Consumers
   * that react to events — not just count them — need that line, otherwise the
   * backlog looks like a burst of brand-new activity on every page load.
   *
   * `undefined` until the first snapshot lands.
   */
  backlogCursor: number | undefined;
}

/**
 * Live event feed: snapshot for the backlog, stream for the tail.
 *
 * The stream carries whole snapshots, so `nextCursor` arrives with every frame
 * and resume-after-reconnect is just replaying it.
 */
export function useLiveEvents(): LiveFeed {
  const [events, setEvents] = useState<WebLiveEvent[]>([]);
  const [status, setStatus] = useState<LiveStatus>("connecting");
  const [recoveries, setRecoveries] = useState(0);
  const [backlogCursor, setBacklogCursor] = useState<number | undefined>(undefined);
  const cursor = useRef<number | undefined>(undefined);

  useEffect(() => {
    const controller = new AbortController();
    let attempts = 0;
    let timer: number | undefined;

    const append = (incoming: WebLiveEvent[]) => {
      if (incoming.length === 0) {
        return;
      }
      setEvents((current) => [...incoming].reverse().concat(current).slice(0, FEED_LIMIT));
    };

    const loadSnapshot = async () => {
      const snapshot = await apiClient.liveSnapshot();
      cursor.current = snapshot.nextCursor;
      setBacklogCursor(snapshot.nextCursor);
      setEvents([...snapshot.events].reverse().slice(0, FEED_LIMIT));
    };

    const connect = async () => {
      try {
        if (cursor.current === undefined) {
          await loadSnapshot();
        }
        setStatus("live");
        await apiClient.liveStream(
          cursor.current,
          (frame) => {
            attempts = 0;
            if (isCursorExpired(frame)) {
              // The buffer compacted past where we were reading. The events we
              // hold may no longer be contiguous with the server's, so drop
              // them and take a clean snapshot — the recovery the backend asks
              // for in `recovery`.
              cursor.current = undefined;
              setEvents([]);
              setRecoveries((count) => count + 1);
              return;
            }
            cursor.current = frame.nextCursor;
            append(frame.events);
          },
          controller.signal,
        );
      } catch (error) {
        if (controller.signal.aborted) {
          return;
        }
        console.warn("live event stream failed", error);
      }
      if (controller.signal.aborted) {
        return;
      }
      attempts += 1;
      if (attempts > MAX_RECONNECTS) {
        setStatus("offline");
        return;
      }
      setStatus("connecting");
      timer = window.setTimeout(() => void connect(), RECONNECT_BASE_MS * attempts);
    };

    void connect();
    return () => {
      controller.abort();
      if (timer !== undefined) {
        window.clearTimeout(timer);
      }
    };
  }, []);

  return { events, status, recoveries, backlogCursor };
}
