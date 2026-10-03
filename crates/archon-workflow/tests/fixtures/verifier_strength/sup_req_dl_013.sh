set -eu
cargo build -q --bin archon
bin="$PWD/target/debug/archon"
T=$(mktemp -d)
cleanup() { if [ -n "${SERVER_PID:-}" ]; then kill "$SERVER_PID" 2>/dev/null || true; fi; rm -rf "$T"; }
trap cleanup EXIT
mkdir -p "$T/crypto-root"
python3 - "$T" <<'PYFIXTURE'
import json, pathlib, sys
srv = pathlib.Path(sys.argv[1]) / 'srv'
srv.mkdir()
rows = [{'date': '2026-06-%02d' % day, 'open': 100.0 + i, 'high': 102.0 + i, 'low': 99.0 + i, 'close': 101.0 + i, 'volume': 1000.0 + 100 * i} for i, day in enumerate(range(1, 6))]
(srv / 'crypto-ok.json').write_text(json.dumps({'id': 'e2e', 'results': rows, 'provider': 'polygon', 'chart': None}))
PYFIXTURE
python3 - "$T" <<'PYSERVER' &
import pathlib, sys
from http.server import HTTPServer, BaseHTTPRequestHandler
srv = pathlib.Path(sys.argv[1]) / 'srv'
class Handler(BaseHTTPRequestHandler):
    def do_GET(self):
        if self.path.startswith('/crypto'):
            self.send_response(200)
            self.end_headers()
            self.wfile.write((srv / 'crypto-ok.json').read_bytes())
            return
        self.send_response(404)
        self.end_headers()
    def log_message(self, *args):
        pass
server = HTTPServer(('127.0.0.1', 0), Handler)
pathlib.Path(sys.argv[1], 'port.txt').write_text(str(server.server_port))
server.serve_forever()
PYSERVER
SERVER_PID=$!
i=0
while [ "$i" -lt 50 ] && ! [ -s "$T/port.txt" ]; do sleep 0.1; i=$((i + 1)); done
if ! [ -s "$T/port.txt" ]; then echo 'SUP-REQ-DL-013 false: the local polygon stand-in server never became ready' >&2; exit 1; fi
PORT=$(cat "$T/port.txt")
if ! OPENBB_API_URL="http://127.0.0.1:$PORT/crypto" POLYGON_API_KEY=e2e-key "$bin" trading data fetch-native --target "$T/crypto-root" --provider polygon --symbol BTCUSDT --timeframe 1D --start 2026-06-01 --end 2026-06-05 --dataset-id polygon-BTCUSDT-1D-raw > "$T/crypto.json" 2> "$T/crypto.err"; then
  echo 'SUP-REQ-DL-013 false: the polygon BTCUSDT crypto fetch-native ingest failed, so no REQ-DL-013 crypto dataset exists to inspect' >&2
  cat "$T/crypto.err" >&2 || true
  exit 1
fi
python3 - "$T" <<'PYVERIFY'
import json, pathlib, sys
T = pathlib.Path(sys.argv[1])
ROOT = T / 'crypto-root'
DATA = ROOT / '.archon' / 'trading-lab' / 'data'
def fail(message):
    raise SystemExit('SUP-REQ-DL-013 false: ' + message)
report = json.loads((T / 'crypto.json').read_text())
if report.get('provider') != 'polygon' or report.get('symbol') != 'BTCUSDT' or report.get('dataset_id') != 'polygon-BTCUSDT-1D-raw' or report.get('can_fetch') is not True:
    fail('the fetch-native command did not complete the crypto ingest: %s' % json.dumps(report)[:300])
base = DATA / 'datasets' / 'polygon-BTCUSDT-1D-raw'
versions = sorted(d for d in base.iterdir() if d.is_dir() and (d / 'metadata.json').is_file()) if base.is_dir() else []
if len(versions) != 1:
    fail('expected exactly one stored crypto dataset version for polygon-BTCUSDT-1D-raw, found %d' % len(versions))
version_dir = versions[0]
meta = json.loads((version_dir / 'metadata.json').read_text())
if meta.get('asset_class') != 'crypto':
    fail('metadata.json asset_class is %r, not the crypto class REQ-DL-013 keys on' % (meta.get('asset_class'),))
if meta.get('canonical_instrument') != 'BTCUSDT':
    fail('metadata.json canonical_instrument is %r, not BTCUSDT' % (meta.get('canonical_instrument'),))
if meta.get('session') != '24x7':
    fail('metadata.json session is %r, not the 24x7 crypto session assumption' % (meta.get('session'),))
if meta.get('timezone') != 'UTC':
    fail('metadata.json timezone is %r, not UTC for the around-the-clock crypto market' % (meta.get('timezone'),))
if meta.get('provider') != 'polygon' or meta.get('provider_symbol') != 'BTCUSD':
    fail('metadata.json exchange source is %r/%r, not the polygon aggregated BTCUSD pair' % (meta.get('provider'), meta.get('provider_symbol')))
endpoint = (meta.get('source') or {}).get('url_or_endpoint') or ''
if 'crypto' not in endpoint:
    fail('metadata.json source.url_or_endpoint %r does not name the crypto market source' % (endpoint,))
validation = json.loads((version_dir / 'validation.json').read_text())
if validation.get('schema_version') != 'archon-trading-validation-v1':
    fail('the stored validation.json schema is %r' % (validation.get('schema_version'),))
if validation.get('dataset_id') != 'polygon-BTCUSDT-1D-raw' or validation.get('version') != meta.get('version'):
    fail('the stored validation report is stale or bound to another dataset')
evidence = validation.get('session_calendar_evidence')
if not isinstance(evidence, dict):
    fail('the stored validation report carries no session_calendar_evidence')
if evidence.get('session') != '24x7' or evidence.get('calendar') != 'continuous_24x7' or evidence.get('timezone') != 'UTC':
    fail('the validation report does not derive the continuous_24x7 calendar from the recorded crypto session: %r' % (evidence,))
paths = meta.get('paths') or {}
notes_rel = paths.get('provider_notes')
if not notes_rel:
    fail('metadata.json records no provider_notes artifact')
notes = (ROOT / notes_rel).read_text()
if 'session=24x7' not in notes:
    fail('the stored provider notes do not record the 24x7 session assumption: %r' % (notes[:200],))
request_rel = paths.get('raw_request')
if not request_rel:
    fail('metadata.json records no raw_request artifact')
raw_request = json.loads((ROOT / request_rel).read_text())
if raw_request.get('openbb_provider') != 'polygon':
    fail('the stored raw request does not name the polygon exchange source')
if 'crypto' not in str(raw_request.get('endpoint') or ''):
    fail('the stored raw request endpoint %r does not name the crypto market' % (raw_request.get('endpoint'),))
if (raw_request.get('params') or {}).get('symbol') != 'BTCUSD':
    fail('the stored raw request does not carry the aggregated BTCUSD pair symbol')
print('SUP-REQ-DL-013 verified: the polygon BTCUSDT crypto dataset records the 24x7 session assumption in metadata, the continuous_24x7 session-calendar evidence in its stored validation report, session=24x7 in its provider notes, and the exchange source (polygon aggregated BTCUSD pair via the crypto market endpoint) in metadata and the stored raw request')
PYVERIFY
