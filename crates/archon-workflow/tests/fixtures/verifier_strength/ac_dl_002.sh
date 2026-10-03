set -eu
cargo build -q --bin archon
bin="$PWD/target/debug/archon"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
cap() {
  out="$1"
  shift
  if ! "$bin" trading data capability "$@" > "$out" 2> "$out.err"; then
    echo "AC-DL-002: 'trading data capability $*' exited non-zero; the capability subcommand is unwired or broken" >&2
    cat "$out.err" >&2 || true
    exit 1
  fi
}
for n in 1 2 3 4 5 6 7; do mkdir -p "$tmp/t$n/.archon/trading-lab/data"; done
cap "$tmp/stooq-1d.json" --provider stooq --symbol SPY --timeframe 1D --target "$tmp/t1"
cap "$tmp/yfinance-1d.json" --provider yfinance --symbol SPY --timeframe 1D --target "$tmp/t2"
cap "$tmp/stooq-240.json" --provider stooq --symbol SPY --timeframe 240 --target "$tmp/t3"
cap "$tmp/yfinance-240.json" --provider yfinance --symbol SPY --timeframe 240 --target "$tmp/t4"
if ! env -u POLYGON_API_KEY "$bin" trading data capability --provider openbb_polygon --symbol SPY --timeframe 1D --target "$tmp/t5" > "$tmp/openbb-nocred.json" 2> "$tmp/openbb-nocred.err"; then
  echo "AC-DL-002: openbb_polygon credential-refusal lane exited non-zero" >&2
  cat "$tmp/openbb-nocred.err" >&2 || true
  exit 1
fi
cap "$tmp/unknown-provider.json" --provider definitely_not_a_provider --symbol SPY --timeframe 1D --target "$tmp/t6"
cap "$tmp/unknown-symbol.json" --provider stooq --symbol ZZZZ --timeframe 1D --target "$tmp/t7"
python3 - "$tmp" <<'PY'
import json, os, sys

tmp = sys.argv[1]

def load(name):
    with open(os.path.join(tmp, name)) as handle:
        return json.load(handle)

def fail(lane, message):
    raise SystemExit("AC-DL-002 false on lane %s: %s" % (lane, message))

SECTION22 = [
    "provider", "canonical_instrument", "provider_symbol", "timeframe",
    "can_fetch", "native_interval", "current_snapshot_supported",
    "historical_supported", "requires_credentials", "unavailable_reason",
    "checked_at",
]

def shaped(lane, name, provider, symbol, timeframe):
    report = load(name)
    for field in SECTION22:
        if field not in report:
            fail(lane, "command printed a legacy shape; section-22 field %r missing" % field)
    if report["provider"] != provider:
        fail(lane, "report names provider %r, requested %r" % (report["provider"], provider))
    if report["canonical_instrument"] != symbol:
        fail(lane, "report names canonical_instrument %r, requested %r" % (report["canonical_instrument"], symbol))
    if report["timeframe"] != timeframe:
        fail(lane, "report names timeframe %r, requested %r" % (report["timeframe"], timeframe))
    if not isinstance(report["provider_symbol"], str) or not report["provider_symbol"].strip():
        fail(lane, "provider_symbol is empty")
    if not isinstance(report["checked_at"], str) or not report["checked_at"].strip():
        fail(lane, "checked_at is empty")
    if not isinstance(report["can_fetch"], bool):
        fail(lane, "can_fetch is not a boolean")
    reason = report["unavailable_reason"]
    if not report["can_fetch"] and (not isinstance(reason, str) or not reason.strip()):
        fail(lane, "can_fetch=false carries no exact unavailable_reason")
    return report

def expect_native(lane, report):
    if not (report["can_fetch"] and report["native_interval"] and report["historical_supported"]):
        fail(lane, "native lane did not report exact native support")

def refusal(lane, report, needles):
    if report["native_interval"] or report["historical_supported"]:
        fail(lane, "refused lane still claims native or historical support")
    reason = report["unavailable_reason"] or ""
    for needle in needles:
        if needle not in reason:
            fail(lane, "unavailable_reason %r does not name %r" % (reason, needle))

expect_native("stooq SPY 1D", shaped("stooq SPY 1D", "stooq-1d.json", "stooq", "SPY", "1D"))
expect_native("yfinance SPY 1D", shaped("yfinance SPY 1D", "yfinance-1d.json", "yfinance", "SPY", "1D"))
refusal("stooq SPY 240", shaped("stooq SPY 240", "stooq-240.json", "stooq", "SPY", "240"), ["stooq", "240"])
refusal("yfinance SPY 240", shaped("yfinance SPY 240", "yfinance-240.json", "yfinance", "SPY", "240"), ["yfinance", "240"])
openbb = shaped("openbb_polygon SPY 1D without credentials", "openbb-nocred.json", "openbb_polygon", "SPY", "1D")
refusal("openbb_polygon SPY 1D without credentials", openbb, ["POLYGON_API_KEY"])
if openbb["requires_credentials"] is not True:
    fail("openbb_polygon SPY 1D without credentials", "missing-credential refusal does not flag requires_credentials")
refusal("unknown provider", shaped("unknown provider", "unknown-provider.json", "definitely_not_a_provider", "SPY", "1D"), ["definitely_not_a_provider"])
refusal("unknown symbol", shaped("unknown symbol", "unknown-symbol.json", "stooq", "ZZZZ", "1D"), ["ZZZZ"])
print("AC-DL-002 verified: the capability command reports section-22 native support or exact unavailable reasons for all seven provider/symbol/timeframe lanes")
PY
