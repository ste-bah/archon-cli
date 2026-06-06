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

if [ -f "$HOME/.profile" ]; then
    # shellcheck disable=SC1090
    . "$HOME/.profile" >/dev/null 2>&1 || true
fi

TARGET=$(cd "$TARGET" && pwd)
TOOLS_DIR="$TARGET/.archon/tools"
TV_DIR="$TOOLS_DIR/tradingview-mcp"
OPENBB_VENV="$TOOLS_DIR/openbb-venv"
MCP_FILE="$TARGET/.mcp.json"

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
    state "$OPENBB_VENV/bin/openbb-api"
    state "$MCP_FILE"
}

if [ "$CHECK" = true ]; then
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
    need_bin python3
    if [ ! -x "$OPENBB_VENV/bin/python" ]; then
        run python3 -m venv "$OPENBB_VENV"
    fi
    run "$OPENBB_VENV/bin/python" -m pip install --upgrade pip
    run "$OPENBB_VENV/bin/python" -m pip install openbb openbb-platform-api
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
    Darwin) exec "$ROOT/.archon/tools/tradingview-mcp/scripts/launch_tv_debug_mac.sh" "$PORT" ;;
    Linux) exec "$ROOT/.archon/tools/tradingview-mcp/scripts/launch_tv_debug_linux.sh" "$PORT" ;;
    *) echo "Start TradingView manually with --remote-debugging-port=$PORT" >&2; exit 1 ;;
esac
EOF
    chmod +x "$TARGET/scripts/start-tradingview-cdp.sh"
fi

if [ "$SKIP_OPENBB" != true ]; then
    cat > "$TARGET/scripts/start-openbb-api.sh" <<'EOF'
#!/bin/sh
set -eu
ROOT=$(CDPATH= cd "$(dirname "$0")/.." && pwd)
HOST="${OPENBB_HOST:-127.0.0.1}"
PORT="${OPENBB_PORT:-6900}"
exec "$ROOT/.archon/tools/openbb-venv/bin/openbb-api" --host "$HOST" --port "$PORT"
EOF
    chmod +x "$TARGET/scripts/start-openbb-api.sh"
fi

echo "setup-trading-tools: complete"
check_status
