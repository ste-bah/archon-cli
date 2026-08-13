use super::*;

type RowFaultCase = (&'static str, fn(&mut OhlcvBar), &'static [&'static str]);

const ROW_FAULT_CASES: [RowFaultCase; 12] = [
    (
        "positive infinity",
        |bar| bar.open = f64::INFINITY,
        &["ohlcv.finite_numbers", "ohlcv.ohlc_sanity"],
    ),
    (
        "negative infinity",
        |bar| bar.open = f64::NEG_INFINITY,
        &[
            "ohlcv.finite_numbers",
            "ohlcv.nonnegative_prices",
            "ohlcv.ohlc_sanity",
        ],
    ),
    (
        "not a number",
        |bar| bar.open = f64::NAN,
        &["ohlcv.finite_numbers", "ohlcv.ohlc_sanity"],
    ),
    (
        "negative open",
        |bar| bar.open = -1.0,
        &["ohlcv.nonnegative_prices", "ohlcv.ohlc_sanity"],
    ),
    (
        "negative high",
        |bar| bar.high = -1.0,
        &["ohlcv.nonnegative_prices", "ohlcv.ohlc_sanity"],
    ),
    (
        "negative low",
        |bar| bar.low = -1.0,
        &["ohlcv.nonnegative_prices", "ohlcv.ohlc_sanity"],
    ),
    (
        "negative close",
        |bar| bar.close = -1.0,
        &["ohlcv.nonnegative_prices", "ohlcv.ohlc_sanity"],
    ),
    (
        "high below low",
        |bar| bar.high = bar.low - 1.0,
        &["ohlcv.ohlc_sanity"],
    ),
    (
        "open below low",
        |bar| bar.open = bar.low - 1.0,
        &["ohlcv.ohlc_sanity"],
    ),
    (
        "open above high",
        |bar| bar.open = bar.high + 1.0,
        &["ohlcv.ohlc_sanity"],
    ),
    (
        "close above high",
        |bar| bar.close = bar.high + 1.0,
        &["ohlcv.ohlc_sanity"],
    ),
    (
        "close below low",
        |bar| bar.close = bar.low - 1.0,
        &["ohlcv.ohlc_sanity"],
    ),
];

pub(super) fn run() {
    for (case, mutate_middle, failed_checks) in ROW_FAULT_CASES {
        let mut bars = vec![
            bar("2026-01-01T00:00:00Z", 10.0, 100.0),
            bar("2026-01-02T00:00:00Z", 11.0, 110.0),
            bar("2026-01-03T00:00:00Z", 12.0, 120.0),
        ];
        mutate_middle(&mut bars[1]);
        bars[2].low = -1.0;
        let metadata = complete_metadata(&bars);
        let report = validation_report(&metadata, &bars, metadata.created_at.clone());

        assert_eq!(report.summary.row_count, 3, "{case}");
        assert_eq!(report.summary.bad_ohlc_count, 2, "{case}");
        assert_eq!(report.summary.missing_volume_count, 0, "{case}");
        for check_id in failed_checks {
            assert_failed_check(&report, check_id);
        }
    }
}
