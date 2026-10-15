#!/bin/sh
# setup-trading-tools.sh — install project-local TradingView MCP + OpenBB tools.
#
# Usage:
#   scripts/setup-trading-tools.sh --target /path/to/project
#   scripts/setup-trading-tools.sh --target /path/to/project --check
#   scripts/setup-trading-tools.sh --target /path/to/project --skip-openbb
#
# Installs external tools under <project>/.archon/tools so the source tree stays
# clean and the project-local .mcp.json can point Archon at the pinned server.

set -eu
if [ -n "${HOME:-}" ] && [ -f "$HOME/.profile" ]; then
    set +eu
    . "$HOME/.profile" >/dev/null 2>&1
    profile_status=$?
    set -eu
    if [ "$profile_status" -ne 0 ]; then
        echo "setup-trading-tools.sh: warning: could not fully source profile: $HOME/.profile" >&2
    fi
fi

TARGET="${PWD:-.}"
CHECK=false
SKIP_TRADINGVIEW=false
SKIP_OPENBB=false
DRY_RUN=false

TV_REPO="https://github.com/tradesdontlie/tradingview-mcp.git"
TV_DIR=""
OPENBB_VENV=""

while [ $# -gt 0 ]; do
    case "$1" in
        --target)
            if [ "$#" -lt 2 ] || [ -z "$2" ]; then
                echo "setup-trading-tools.sh: --target requires a path" >&2
                exit 1
            fi
            shift
            TARGET="$1"
            ;;
        --check)
            CHECK=true
            ;;
        --skip-tradingview)
            SKIP_TRADINGVIEW=true
            ;;
        --skip-openbb)
            SKIP_OPENBB=true
            ;;
        --dry-run)
            DRY_RUN=true
            ;;
        --help|-h)
            sed -n '1,18p' "$0" | sed 's/^# //; s/^#//'
            exit 0
            ;;
        *)
            echo "setup-trading-tools.sh: unknown flag: $1" >&2
            exit 1
            ;;
    esac
    shift
done

TARGET=$(cd "$TARGET" && pwd)
TOOLS_DIR="$TARGET/.archon/tools"
TV_DIR="$TOOLS_DIR/tradingview-mcp"
OPENBB_VENV="$TOOLS_DIR/openbb-venv"
MCP_FILE="$TARGET/.mcp.json"
REQUIRES_PYTHON='>=3.10,<4'

run() {
    if [ "$DRY_RUN" = true ] || [ "$CHECK" = true ]; then
        echo "[dry-run] $*"
    else
        echo "+ $*"
        "$@"
    fi
}

need_bin() {
    if ! command -v "$1" >/dev/null 2>&1; then
        echo "setup-trading-tools.sh: missing required binary: $1" >&2
        echo "  Run: scripts/install-system-deps.sh --with-trading-tools" >&2
        exit 2
    fi
}

load_openbb_specifier() {
    if [ -d "$OPENBB_VENV" ]; then
        spec=$(find "$OPENBB_VENV" -type f -path '*/openbb-*.dist-info/METADATA' -exec awk 'index($0, "Requires-Python: ")==1 {sub(/^Requires-Python: /, ""); print; exit}' {} \; 2>/dev/null | head -n 1)
        if [ -n "$spec" ]; then REQUIRES_PYTHON="$spec"; fi
    fi
}

python_satisfies_specifier() (
    candidate=$1
    probe_base=$(mktemp "${TMPDIR:-/tmp}/setup-trading-tools-python-probe.XXXXXX")
    probe_file="$probe_base.py"
    mv "$probe_base" "$probe_file"
    trap 'rm -f "$probe_file"' EXIT
    cat > "$probe_file" <<'PYTHON'
import os, re, sys
spec = os.environ["ARCHON_SETUP_PYTHON_SPECIFIER"]
version = sys.version_info
release = tuple(version[:3])
pre = {"alpha": (0, version.serial), "beta": (1, version.serial), "candidate": (2, version.serial)}.get(version.releaselevel, (3, 0))

def compare(left, right):
    a, a_pre = left
    b, b_pre = right
    a = tuple(a[:next((i for i in range(len(a) - 1, -1, -1) if a[i] != 0), -1) + 1])
    b = tuple(b[:next((i for i in range(len(b) - 1, -1, -1) if b[i] != 0), -1) + 1])
    width = max(len(a), len(b))
    a, b = a + (0,) * (width - len(a)), b + (0,) * (width - len(b))
    if a != b: return (a > b) - (a < b)
    return (a_pre > b_pre) - (a_pre < b_pre)

def check_clause(clause):
    match = re.fullmatch(r"(>=|<=|==|!=|~=|>|<)\s*(\d+(?:\.\d+)*)(a|b|rc)?(\d+)?(\.\*)?", clause.strip(), re.I)
    if not match:
        op = re.match(r"([^\s\d]+)", clause.strip())
        return False, "unknown operator %s" % (op.group(1) if op else "(invalid clause)")
    op, raw, pre_label, pre_serial, wildcard = match.groups()
    if pre_label and pre_serial is None:
        return False, "invalid pre-release clause"
    wanted_pre = (({"a": 0, "b": 1, "rc": 2}[(pre_label or "").lower()], int(pre_serial)) if pre_label else (3, 0))
    wanted = (tuple(map(int, raw.split("."))), wanted_pre)
    order = compare((release, pre), wanted)
    if wildcard and op in ("==", "!="):
        prefix = wanted[0]
        equal = release[:len(prefix)] == prefix
        return (equal if op == "==" else not equal), None
    if wildcard:
        return False, "wildcard is only valid with == or !="
    if op == ">=": return order >= 0, None
    if op == ">": return order > 0, None
    if op == "<=": return order <= 0, None
    if op == "<": return order < 0, None
    if op == "==": return order == 0, None
    if op == "!=": return order != 0, None
    if op == "~=":
        if len(wanted[0]) < 2: return False, "invalid compatible-release clause"
        upper = list(wanted[0][:-1])
        upper[-1] += 1
        upper_bound = (tuple(upper), (3, 0))
        return order >= 0 and compare(current, upper_bound) < 0, None

clauses = spec.split(",")
for clause in clauses:
    _, error = check_clause(clause)
    if error:
        print("%s (treated as unsatisfied)" % error)
        raise SystemExit(2)
for clause in clauses:
    passed, _ = check_clause(clause)
    if not passed:
        raise SystemExit(1)
raise SystemExit(0)
PYTHON
    ARCHON_SETUP_PYTHON_SPECIFIER="$REQUIRES_PYTHON" "$candidate" "$probe_file" 2>&1
)

find_python() {
    unknown_operator=''
    for candidate in python3.20 python3.19 python3.18 python3.17 python3.16 python3.15 \
        python3.14 python3.13 python3.12 python3.11 python3.10 python3; do
        if ! command -v "$candidate" >/dev/null 2>&1; then continue; fi
        if output=$(python_satisfies_specifier "$candidate"); then
            PYTHON_BASE="$candidate"
            return 0
        else
            status=$?
            if [ "$status" -eq 2 ]; then unknown_operator=$output; fi
        fi
    done
    echo "setup-trading-tools.sh: no Python interpreter satisfies OpenBB Requires-Python specifier '$REQUIRES_PYTHON'. ${unknown_operator}" >&2
    return 1
}

prepare_venv_rebuild() {
    if [ -L "$OPENBB_VENV" ]; then
        link_target=$(readlink "$OPENBB_VENV" 2>/dev/null || echo unknown)
        echo "setup-trading-tools.sh: refusing to rebuild symlinked OpenBB venv: $OPENBB_VENV -> $link_target" >&2
        return 1
    fi
    if [ -e "$OPENBB_VENV" ] && [ ! -f "$OPENBB_VENV/pyvenv.cfg" ]; then
        if [ -d "$OPENBB_VENV" ] && [ -z "$(find "$OPENBB_VENV" -mindepth 1 -maxdepth 1 -print -quit 2>/dev/null)" ]; then
            return 0
        fi
        echo "setup-trading-tools.sh: refusing to rebuild non-venv directory with contents: $OPENBB_VENV (missing pyvenv.cfg)" >&2
        return 1
    fi
}

state() {
    if [ -e "$1" ]; then
        echo "present: $1"
    else
        echo "missing: $1"
    fi
}

check_status() {
    echo "setup-trading-tools: target=$TARGET"
    command -v node >/dev/null 2>&1 && node --version || echo "node: missing"
    command -v npm >/dev/null 2>&1 && npm --version || echo "npm: missing"
    command -v python3 >/dev/null 2>&1 && python3 --version || echo "python3: missing"
    command -v git >/dev/null 2>&1 && git --version || echo "git: missing"
    state "$TV_DIR/src/server.js"
    state "$TV_DIR/src/cli/index.js"
    if [ -x "$OPENBB_VENV/bin/python" ] && "$OPENBB_VENV/bin/python" -c 'import openbb' >/dev/null 2>&1; then
        echo "OpenBB venv: healthy ($OPENBB_VENV/bin/python)"
    elif [ -d "$OPENBB_VENV" ]; then
        echo "OpenBB venv: broken ($OPENBB_VENV)"
    else
        echo "OpenBB venv: missing ($OPENBB_VENV)"
    fi
    state "$OPENBB_VENV/bin/openbb-api"
    state "$MCP_FILE"
}

if [ "$CHECK" = true ] || [ "$DRY_RUN" = true ]; then
    check_status
    exit 0
fi

mkdir -p "$TOOLS_DIR"

if [ "$SKIP_TRADINGVIEW" != true ]; then
    need_bin git
    need_bin node
    need_bin npm
    if [ -d "$TV_DIR/.git" ]; then
        run git -C "$TV_DIR" pull --ff-only
    else
        run git clone "$TV_REPO" "$TV_DIR"
    fi
    run npm --prefix "$TV_DIR" install --omit=dev
    chmod +x "$TV_DIR/scripts/"*.sh "$TV_DIR/src/cli/index.js" 2>/dev/null || true
fi

if [ "$SKIP_OPENBB" != true ]; then
    if [ -x "$OPENBB_VENV/bin/python" ] && "$OPENBB_VENV/bin/python" -c 'import openbb' >/dev/null 2>&1; then
        :
    else
        load_openbb_specifier
        find_python
        prepare_venv_rebuild
        run "$PYTHON_BASE" -m venv --clear "$OPENBB_VENV"
    fi
    run "$OPENBB_VENV/bin/python" -m pip install --upgrade pip
    run "$OPENBB_VENV/bin/python" -m pip install openbb openbb-platform-api
    if ! "$OPENBB_VENV/bin/python" -c 'import openbb' >/dev/null 2>&1; then
        echo "setup-trading-tools.sh: OpenBB environment is still broken after installation: $OPENBB_VENV" >&2
        exit 1
    fi
fi

if [ "$SKIP_TRADINGVIEW" != true ]; then
    need_bin node
    node - "$MCP_FILE" "$TV_DIR/src/server.js" <<'NODE'
const fs = require('fs');
const file = process.argv[2];
const serverPath = process.argv[3];
let config = {};
if (fs.existsSync(file)) {
  config = JSON.parse(fs.readFileSync(file, 'utf8'));
}
config.mcpServers = config.mcpServers || {};
config.mcpServers.tradingview = {
  command: "node",
  args: [serverPath],
  env: {
    TV_CDP_HOST: "localhost",
    TV_CDP_PORT: "9222"
  },
  toolPolicy: {
    trustServerHints: false,
    toolPermissions: {
      tv_health_check: "safe",
      tv_discover: "safe",
      tv_ui_state: "safe",
      chart_get_state: "safe",
      quote_get: "safe",
      data_get_ohlcv: "safe",
      data_get_study_values: "safe",
      data_get_pine_lines: "safe",
      data_get_pine_labels: "safe",
      data_get_pine_tables: "safe",
      data_get_pine_boxes: "safe",
      capture_screenshot: "safe",
      pine_analyze: "safe",
      pine_check: "safe",
      pine_get_errors: "safe",
      pine_get_console: "safe",
      pine_get_source: "risky",
      pine_set_source: "risky",
      pine_compile: "risky",
      pine_smart_compile: "risky",
      pine_save: "risky",
      alert_create: "risky",
      alert_delete: "risky",
      replay_trade: "risky",
      tv_launch: "risky"
    }
  }
};
fs.writeFileSync(file, JSON.stringify(config, null, 2) + "\n");
NODE
fi

mkdir -p "$TARGET/scripts"
if [ "$SKIP_TRADINGVIEW" != true ]; then
    cat > "$TARGET/scripts/start-tradingview-cdp.sh" <<'EOF'
#!/bin/sh
set -eu
ROOT=$(CDPATH= cd "$(dirname "$0")/.." && pwd)
PORT="${1:-9222}"
case "$(uname -s 2>/dev/null || echo unknown)" in
    Darwin) LAUNCHER="$ROOT/.archon/tools/tradingview-mcp/scripts/launch_tv_debug_mac.sh" ;;
    Linux) LAUNCHER="$ROOT/.archon/tools/tradingview-mcp/scripts/launch_tv_debug_linux.sh" ;;
    *) echo "TradingView CDP launcher is unavailable on this platform; install the external TradingView launcher." >&2; exit 1 ;;
esac
if [ ! -f "$LAUNCHER" ]; then
    echo "TradingView CDP launcher is missing: $LAUNCHER. Install the external TradingView launcher project." >&2
    exit 1
fi
exec "$LAUNCHER" "$PORT"
EOF
    chmod +x "$TARGET/scripts/start-tradingview-cdp.sh"
    cat > "$TARGET/scripts/start-tradingview-cdp.ps1" <<'EOF'
$ErrorActionPreference = 'Stop'
$Root = Split-Path -Parent (Split-Path -Parent $MyInvocation.MyCommand.Path)
$Port = if ($args.Count -gt 0) { $args[0] } else { '9222' }
$Launcher = Join-Path $Root '.archon/tools/tradingview-mcp/scripts/launch_tv_debug_windows.ps1'
if (-not (Test-Path -LiteralPath $Launcher -PathType Leaf)) { throw "TradingView CDP launcher is missing: $Launcher. Install the external TradingView launcher project." }
& $Launcher $Port
exit $LASTEXITCODE
EOF
fi

if [ "$SKIP_OPENBB" != true ]; then
    cat > "$TARGET/scripts/start-openbb-api.sh" <<'EOF'
#!/bin/sh
set -eu
ROOT=$(CDPATH= cd "$(dirname "$0")/.." && pwd)
HOST="${OPENBB_HOST:-127.0.0.1}"
PORT="${OPENBB_PORT:-6900}"
PYTHON="$ROOT/.archon/tools/openbb-venv/bin/python"
API="$ROOT/.archon/tools/openbb-venv/bin/openbb-api"
if [ ! -x "$PYTHON" ]; then echo "OpenBB interpreter is missing: $PYTHON. Run archon trading setup to repair the environment." >&2; exit 1; fi
if ! "$PYTHON" -c 'import openbb' >/dev/null 2>&1; then echo "OpenBB environment is broken. Run archon trading setup to recreate it." >&2; exit 1; fi
if [ ! -x "$API" ]; then echo "OpenBB API launcher is missing: $API. Run archon trading setup to install packages." >&2; exit 1; fi
exec "$API" --host "$HOST" --port "$PORT"
EOF
    chmod +x "$TARGET/scripts/start-openbb-api.sh"
    cat > "$TARGET/scripts/start-openbb-api.ps1" <<'EOF'
$ErrorActionPreference = 'Stop'
$Root = Split-Path -Parent (Split-Path -Parent $MyInvocation.MyCommand.Path)
$HostName = if ($env:OPENBB_HOST) { $env:OPENBB_HOST } else { '127.0.0.1' }
$Port = if ($env:OPENBB_PORT) { $env:OPENBB_PORT } else { '6900' }
$Python = Join-Path $Root '.archon/tools/openbb-venv/Scripts/python.exe'
if (-not (Test-Path -LiteralPath $Python -PathType Leaf)) { throw "OpenBB interpreter is missing: $Python. Run archon trading setup to repair the environment." }
& $Python -c 'import openbb' 2>$null
if ($LASTEXITCODE -ne 0) { throw "OpenBB environment is broken. Run archon trading setup to recreate it." }
$Api = Join-Path $Root '.archon/tools/openbb-venv/Scripts/openbb-api.exe'
if (-not (Test-Path -LiteralPath $Api -PathType Leaf)) { throw "OpenBB API launcher is missing: $Api. Run archon trading setup to install packages." }
& $Api --host $HostName --port $Port
exit $LASTEXITCODE
EOF
fi

echo "setup-trading-tools: complete"
check_status
