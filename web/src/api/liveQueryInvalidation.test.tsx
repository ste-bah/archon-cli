import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, render } from "@testing-library/react";
import type { ReactNode } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { WebLiveEvent } from "./generated/web";
import {
  ALL_LIVE_QUERY_KEYS,
  LIVE_EVENT_QUERY_KEYS,
  LIVE_INVALIDATION_WINDOW_MS,
  collectQueryKeys,
  queryKeysForEvent,
  useLiveQueryInvalidation,
} from "./liveQueryInvalidation";
import type { LiveFeed } from "./useLiveEvents";

function event(cursor: number, eventType: string): WebLiveEvent {
  return { cursor, eventType, summary: `${eventType} ${cursor}`, createdAtMs: cursor };
}

/** The feed shape `useLiveEvents` returns: newest first, backlog cursor set. */
function feed(events: WebLiveEvent[], backlogCursor: number | undefined, recoveries = 0): LiveFeed {
  return { events, status: "live", recoveries, backlogCursor };
}

describe("live event to query key mapping", () => {
  it("maps every event type the server records", () => {
    // Mirrors the `WebLiveManager::record` call sites in
    // `crates/archon-sdk/src/web`. A new site must land here, not silently do
    // nothing.
    expect(Object.keys(LIVE_EVENT_QUERY_KEYS).sort()).toEqual([
      "web.chat.completed",
      "web.chat.failed",
      "web.chat.submitted",
      "web.docs.deleted",
      "web.index.control",
      "web.ingest.finished",
      "web.ingest.started",
      "web.runtime.started",
      "web.upload.stored",
    ]);
  });

  it("routes a completed chat turn to the surfaces the turn writes", () => {
    expect(queryKeysForEvent("web.chat.completed")).toEqual(["cognitive", "learning", "metrics"]);
  });

  it("routes a finished ingest run to the document and corpus surfaces", () => {
    expect(queryKeysForEvent("web.ingest.finished")).toEqual(["ingest", "corpus", "evidence"]);
  });

  it("routes nothing for events with no durable effect on a summary", () => {
    expect(queryKeysForEvent("web.runtime.started")).toEqual([]);
    expect(queryKeysForEvent("web.upload.stored")).toEqual([]);
    expect(queryKeysForEvent("web.chat.submitted")).toEqual([]);
  });

  it("routes nothing for an unknown event type", () => {
    expect(queryKeysForEvent("web.something.new")).toEqual([]);
  });

  it("never claims a surface that has no event", () => {
    // Workflows, pipelines and world are driven by processes the web server
    // does not record events for; they must not appear here.
    expect(ALL_LIVE_QUERY_KEYS).toEqual([
      "cognitive",
      "corpus",
      "evidence",
      "ingest",
      "learning",
      "metrics",
    ]);
  });
});

describe("collectQueryKeys", () => {
  it("ignores backlog events that predate the snapshot cursor", () => {
    const events = [event(7, "web.ingest.finished"), event(6, "web.chat.completed")];
    expect(collectQueryKeys(events, 8, 0)).toEqual([]);
  });

  it("collects events recorded at or after the snapshot cursor", () => {
    const events = [event(8, "web.ingest.finished"), event(7, "web.chat.completed")];
    expect(collectQueryKeys(events, 8, 0)).toEqual(["ingest", "corpus", "evidence"]);
  });

  it("ignores events already processed", () => {
    const events = [event(9, "web.index.control"), event(8, "web.ingest.finished")];
    expect(collectQueryKeys(events, 8, 8)).toEqual(["ingest"]);
  });

  it("collects nothing before the snapshot has landed", () => {
    expect(collectQueryKeys([event(8, "web.ingest.finished")], undefined, 0)).toEqual([]);
  });

  it("dedupes surfaces across events", () => {
    const events = [event(10, "web.docs.deleted"), event(9, "web.ingest.finished")];
    expect(collectQueryKeys(events, 8, 0).sort()).toEqual(["corpus", "evidence", "ingest"]);
  });
});

describe("useLiveQueryInvalidation", () => {
  let queryClient: QueryClient;
  let invalidate: ReturnType<typeof vi.spyOn>;

  function Harness({ value }: { value: LiveFeed }) {
    useLiveQueryInvalidation(value);
    return null;
  }

  function wrapper(children: ReactNode) {
    return <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>;
  }

  function invalidatedKeys(): string[] {
    const calls = invalidate.mock.calls as Array<[{ queryKey: string[] }]>;
    return calls.map((call) => call[0].queryKey[0] as string).sort();
  }

  beforeEach(() => {
    vi.useFakeTimers();
    queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } });
    invalidate = vi.spyOn(queryClient, "invalidateQueries");
  });

  afterEach(() => {
    vi.useRealTimers();
    queryClient.clear();
  });

  it("does not invalidate for the backlog the page load already fetched", () => {
    render(wrapper(<Harness value={feed([event(7, "web.ingest.finished")], 8)} />));
    act(() => {
      vi.advanceTimersByTime(LIVE_INVALIDATION_WINDOW_MS * 4);
    });
    expect(invalidate).not.toHaveBeenCalled();
  });

  it("invalidates the mapped surfaces once a live event arrives", () => {
    const view = render(wrapper(<Harness value={feed([], 8)} />));
    view.rerender(wrapper(<Harness value={feed([event(8, "web.chat.completed")], 8)} />));
    expect(invalidate).not.toHaveBeenCalled();

    act(() => {
      vi.advanceTimersByTime(LIVE_INVALIDATION_WINDOW_MS);
    });
    expect(invalidatedKeys()).toEqual(["cognitive", "learning", "metrics"]);
  });

  it("coalesces a burst inside one window into one invalidation per surface", () => {
    const view = render(wrapper(<Harness value={feed([], 8)} />));
    const burst: WebLiveEvent[] = [];
    for (let cursor = 8; cursor < 48; cursor += 1) {
      // Newest first, as `useLiveEvents` keeps them.
      burst.unshift(event(cursor, cursor % 2 === 0 ? "web.chat.completed" : "web.ingest.finished"));
      view.rerender(wrapper(<Harness value={feed([...burst], 8)} />));
    }
    expect(invalidate).not.toHaveBeenCalled();

    act(() => {
      vi.advanceTimersByTime(LIVE_INVALIDATION_WINDOW_MS);
    });

    // 40 events across six distinct surfaces: six requests, not 120.
    expect(invalidate).toHaveBeenCalledTimes(6);
    expect(invalidatedKeys()).toEqual([
      "cognitive",
      "corpus",
      "evidence",
      "ingest",
      "learning",
      "metrics",
    ]);
  });

  it("bounds a sustained burst to one invalidation per surface per window", () => {
    const view = render(wrapper(<Harness value={feed([], 8)} />));
    const burst: WebLiveEvent[] = [];
    const gapMs = LIVE_INVALIDATION_WINDOW_MS / 8;
    for (let cursor = 8; cursor < 48; cursor += 1) {
      burst.unshift(event(cursor, cursor % 2 === 0 ? "web.chat.completed" : "web.ingest.finished"));
      view.rerender(wrapper(<Harness value={feed([...burst], 8)} />));
      act(() => {
        vi.advanceTimersByTime(gapMs);
      });
    }
    act(() => {
      vi.advanceTimersByTime(LIVE_INVALIDATION_WINDOW_MS);
    });

    // 40 events spread over 5 windows: at most six requests per window, versus
    // the 120 an invalidate-per-event would have issued.
    const windows = Math.ceil((40 * gapMs) / LIVE_INVALIDATION_WINDOW_MS);
    expect(invalidate.mock.calls.length).toBeLessThanOrEqual(6 * windows);
    expect(invalidate.mock.calls.length).toBeLessThan(120);
  });

  it("invalidates every mapped surface when the cursor expired and left a gap", () => {
    const view = render(wrapper(<Harness value={feed([event(8, "web.chat.completed")], 8)} />));
    act(() => {
      vi.advanceTimersByTime(LIVE_INVALIDATION_WINDOW_MS);
    });
    invalidate.mockClear();

    // `useLiveEvents` drops its events and takes a fresh snapshot on expiry.
    view.rerender(wrapper(<Harness value={feed([], 64, 1)} />));
    act(() => {
      vi.advanceTimersByTime(LIVE_INVALIDATION_WINDOW_MS);
    });
    expect(invalidatedKeys()).toEqual([...ALL_LIVE_QUERY_KEYS]);
  });

  it("stops the pending window when the tree unmounts", () => {
    const view = render(wrapper(<Harness value={feed([], 8)} />));
    view.rerender(wrapper(<Harness value={feed([event(8, "web.chat.completed")], 8)} />));
    view.unmount();
    act(() => {
      vi.advanceTimersByTime(LIVE_INVALIDATION_WINDOW_MS * 4);
    });
    expect(invalidate).not.toHaveBeenCalled();
  });
});
