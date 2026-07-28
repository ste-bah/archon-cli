// Paged native fetch for providers that cap bars per call.
//
// The provider's CLI accepts only `count` (max 500) and returns bars relative
// to the chart's current position, so a span wider than one call must be
// assembled from several. Fetching fewer bars than asked for and registering
// the result anyway is how a dataset comes to look complete while having holes
// in it, so every path here either produces a contiguous series or says
// precisely what it could not serve.

use archon_trading::ohlcv::OhlcvBar;

/// Why a series stopped short of the requested span.
///
/// A short page is ambiguous between a silent provider cap, a closed market,
/// and genuine end-of-history, and no bar count can separate them. One extra
/// request can: ask again from the last bar returned, and let the answer decide.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum SeriesBoundary {
    /// The requested span was served in full.
    Complete,
    /// A further request returned nothing: the provider has no more history.
    /// The dataset is legitimate but shorter than asked for.
    Exhausted {
        served_from: String,
        served_to: String,
    },
}

#[derive(Debug, Clone)]
pub(super) struct PagedSeries {
    pub(super) bars: Vec<OhlcvBar>,
    pub(super) boundary: SeriesBoundary,
    pub(super) pages_fetched: usize,
}

/// The request that continues a series after `last_timestamp`.
///
/// ISOLATED DELIBERATELY. What `ohlcv --count N` returns relative to a prior
/// `scroll <date>` — the N bars ending at that date, starting from it, or
/// centred on it — has not been verified against a live chart, and guessing
/// wrong produces a stitched series with silent holes. When it is verified,
/// this function is the only thing that changes.
pub(super) fn page_request_for(last_timestamp: Option<&str>, count: usize) -> PageRequest {
    PageRequest {
        scroll_to: last_timestamp.map(str::to_string),
        count,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PageRequest {
    /// None for the first page: take the chart wherever it already is.
    pub(super) scroll_to: Option<String>,
    pub(super) count: usize,
}

/// Join pages into one ascending series, dropping the overlap that paging by
/// timestamp necessarily produces.
pub(super) fn stitch(pages: &[Vec<OhlcvBar>]) -> Vec<OhlcvBar> {
    let mut seen = std::collections::BTreeMap::new();
    for page in pages {
        for bar in page {
            // Later pages win: a re-fetched boundary bar should reflect the
            // most recent read rather than the first.
            seen.insert(bar.timestamp.clone(), bar.clone());
        }
    }
    seen.into_values().collect()
}

/// Reject a stitched series whose spacing does not match its timeframe.
///
/// This is the structural check the scroll semantics cannot fake. If a page
/// lands somewhere other than where the loop assumed, the join leaves a gap
/// wider than one bar interval, and that shows up here rather than in a
/// registered dataset. Declared non-trading days are exempt, because a real
/// series legitimately skips them.
pub(super) fn assert_contiguous(
    bars: &[OhlcvBar],
    interval_secs: i64,
    closed_dates: &[String],
) -> Result<(), String> {
    if bars.len() < 2 {
        return Ok(());
    }
    let closed: std::collections::BTreeSet<&str> =
        closed_dates.iter().map(String::as_str).collect();
    let mut previous: Option<i64> = None;
    for bar in bars {
        let epoch = parse_epoch(&bar.timestamp).ok_or_else(|| {
            format!(
                "stitched series has an unparseable timestamp: {}",
                bar.timestamp
            )
        })?;
        if let Some(previous_epoch) = previous {
            if epoch <= previous_epoch {
                return Err(format!(
                    "stitched series is not strictly ascending at {}: {epoch} follows {previous_epoch}",
                    bar.timestamp
                ));
            }
            let step = epoch - previous_epoch;
            if step > interval_secs
                && !spans_only_closed_days(previous_epoch, epoch, interval_secs, &closed)
            {
                return Err(format!(
                    "stitched series has a {step}s hole before {} (bar interval {interval_secs}s); \
                     a page landed somewhere other than where the previous one ended",
                    bar.timestamp
                ));
            }
        }
        previous = Some(epoch);
    }
    Ok(())
}

/// Whether every interval skipped between two bars falls on a declared
/// non-trading day. A gap that does is a market closure, not a paging fault.
fn spans_only_closed_days(
    from_epoch: i64,
    to_epoch: i64,
    interval_secs: i64,
    closed: &std::collections::BTreeSet<&str>,
) -> bool {
    if closed.is_empty() {
        return false;
    }
    let mut cursor = from_epoch + interval_secs;
    while cursor < to_epoch {
        let Some(day) = day_of(cursor) else {
            return false;
        };
        if !closed.contains(day.as_str()) {
            return false;
        }
        cursor += interval_secs;
    }
    true
}

fn parse_epoch(timestamp: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(timestamp)
        .ok()
        .map(|value| value.timestamp())
}

fn day_of(epoch: i64) -> Option<String> {
    chrono::DateTime::from_timestamp(epoch, 0).map(|value| value.format("%Y-%m-%d").to_string())
}

/// Drive the page loop until the requested span is served or the provider runs
/// out of history.
///
/// `fetch` is injected so the loop is testable without a chart. A short page
/// does NOT end the loop on its own: the next request is what distinguishes a
/// silent cap from exhaustion, which is the only way to tell them apart.
pub(super) fn fetch_paged<F>(
    requested_bars: usize,
    per_call_limit: usize,
    max_pages: usize,
    mut fetch: F,
) -> Result<PagedSeries, String>
where
    F: FnMut(PageRequest) -> Result<Vec<OhlcvBar>, String>,
{
    let mut pages: Vec<Vec<OhlcvBar>> = Vec::new();
    let mut last_timestamp: Option<String> = None;

    for page_index in 0..max_pages {
        let request = page_request_for(last_timestamp.as_deref(), per_call_limit);
        let page = fetch(request)?;
        let fetched = page_index + 1;
        if page.is_empty() {
            // The probe answered: nothing further exists. Whatever we already
            // hold is the provider's real extent.
            let bars = stitch(&pages);
            let boundary = if bars.len() >= requested_bars {
                SeriesBoundary::Complete
            } else if let (Some(first), Some(last)) = (bars.first(), bars.last()) {
                SeriesBoundary::Exhausted {
                    served_from: first.timestamp.clone(),
                    served_to: last.timestamp.clone(),
                }
            } else {
                return Err("provider served no bars at all for the requested span".to_string());
            };
            return Ok(PagedSeries {
                bars,
                boundary,
                pages_fetched: fetched,
            });
        }
        last_timestamp = page.last().map(|bar| bar.timestamp.clone());
        pages.push(page);
        if stitch(&pages).len() >= requested_bars {
            let bars = stitch(&pages);
            return Ok(PagedSeries {
                bars,
                boundary: SeriesBoundary::Complete,
                pages_fetched: fetched,
            });
        }
    }
    Err(format!(
        "paged fetch did not reach {requested_bars} bars within {max_pages} pages \
         (served {} so far); refusing to register a partial series as complete",
        stitch(&pages).len()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bar(ts: &str, close: f64) -> OhlcvBar {
        OhlcvBar {
            timestamp: ts.to_string(),
            open: close,
            high: close,
            low: close,
            close,
            volume: 1.0,
        }
    }

    fn day(n: u32, close: f64) -> OhlcvBar {
        bar(&format!("2024-01-{n:02}T00:00:00+00:00"), close)
    }

    const DAY: i64 = 86_400;

    /// A short page is NOT taken as the end. The loop asks again, and only an
    /// empty answer settles it — the difference between a silent cap and real
    /// exhaustion, which no bar count can tell apart.
    #[test]
    fn a_short_page_is_probed_rather_than_believed() {
        let mut calls = 0;
        let series = fetch_paged(4, 2, 5, |_req| {
            calls += 1;
            Ok(match calls {
                1 => vec![day(1, 10.0), day(2, 11.0)],
                2 => vec![day(3, 12.0)], // short: cap, or the end?
                _ => vec![day(4, 13.0)], // the probe proves it was a cap
            })
        })
        .expect("paged fetch");
        assert_eq!(series.bars.len(), 4);
        assert_eq!(series.boundary, SeriesBoundary::Complete);
        assert_eq!(
            series.pages_fetched, 3,
            "the short page must not end the loop"
        );
    }

    /// When the probe comes back empty the series is genuinely short. It stays
    /// usable, but records the span the provider actually served rather than
    /// the one that was asked for.
    #[test]
    fn an_empty_probe_records_the_served_span_not_the_requested_one() {
        let mut calls = 0;
        let series = fetch_paged(10, 2, 5, |_req| {
            calls += 1;
            Ok(if calls == 1 {
                vec![day(1, 10.0), day(2, 11.0)]
            } else {
                vec![]
            })
        })
        .expect("paged fetch");
        assert_eq!(series.bars.len(), 2);
        assert_eq!(
            series.boundary,
            SeriesBoundary::Exhausted {
                served_from: "2024-01-01T00:00:00+00:00".to_string(),
                served_to: "2024-01-02T00:00:00+00:00".to_string(),
            }
        );
    }

    /// Never silently return short: a loop that cannot reach the span within
    /// its page budget fails rather than registering a partial series.
    #[test]
    fn exceeding_the_page_budget_fails_rather_than_registering_a_partial_series() {
        let error = fetch_paged(100, 1, 3, |_req| Ok(vec![day(1, 10.0)])).expect_err("must refuse");
        assert!(
            error.contains("refusing to register a partial series"),
            "{error}"
        );
    }

    /// Paging by timestamp re-reads the boundary bar; the join must not
    /// duplicate it.
    #[test]
    fn stitching_drops_the_overlap_between_pages() {
        let joined = stitch(&[
            vec![day(1, 10.0), day(2, 11.0)],
            vec![day(2, 11.0), day(3, 12.0)],
        ]);
        assert_eq!(joined.len(), 3);
        assert_eq!(joined[1].timestamp, "2024-01-02T00:00:00+00:00");
    }

    /// The structural check the scroll semantics cannot fake. If a page lands
    /// somewhere other than where the loop assumed, the join leaves a hole and
    /// this catches it before anything is registered.
    #[test]
    fn a_hole_from_a_misplaced_page_is_rejected() {
        let bars = vec![day(1, 10.0), day(2, 11.0), day(9, 12.0)];
        let error = assert_contiguous(&bars, DAY, &[]).expect_err("hole must be rejected");
        assert!(error.contains("hole"), "{error}");
        assert!(error.contains("landed somewhere other than"), "{error}");
    }

    /// A real series skips non-trading days, so a declared closure is not a
    /// paging fault. Without this the check would reject every genuine series.
    #[test]
    fn a_gap_across_declared_closed_days_is_not_a_hole() {
        // 2024-01-06 and 07 are a weekend; the series jumps 05 -> 08.
        let bars = vec![day(4, 10.0), day(5, 11.0), day(8, 12.0)];
        let closed = vec!["2024-01-06".to_string(), "2024-01-07".to_string()];
        assert_contiguous(&bars, DAY, &closed).expect("declared closures are legitimate");
    }

    #[test]
    fn an_out_of_order_series_is_rejected() {
        let bars = vec![day(3, 12.0), day(1, 10.0)];
        let error = assert_contiguous(&bars, DAY, &[]).expect_err("must reject");
        assert!(error.contains("not strictly ascending"), "{error}");
    }

    /// Pins the seam that has not been verified against a live chart, so a
    /// change to it is a deliberate act rather than a silent drift.
    #[test]
    fn the_page_request_seam_is_a_single_function() {
        assert_eq!(
            page_request_for(None, 500),
            PageRequest {
                scroll_to: None,
                count: 500
            }
        );
        assert_eq!(
            page_request_for(Some("2024-01-02T00:00:00+00:00"), 500),
            PageRequest {
                scroll_to: Some("2024-01-02T00:00:00+00:00".to_string()),
                count: 500
            }
        );
    }
}
