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
SCRIPT_DIR=${0%/*}
[ "$SCRIPT_DIR" = "$0" ] && SCRIPT_DIR=.
SCRIPT_DIR=$(CDPATH= cd "$SCRIPT_DIR" && pwd -P)
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
PYTHON_CANDIDATES_TEMP=""
PYTHON_FAILURES_TEMP=""
PYTHON_SEEN_TEMP=""
PYTHON_UPDATE_TEMP=""
PYTHON_REPORT_TEMP=""
OPENBB_VENV_READY=false
OPENBB_RESOLVED_VERSION=""
SETUP_PROCESS_ID=$$
cleanup_setup_temp_files() {
    [ "$$" = "$SETUP_PROCESS_ID" ] || return 0
    if [ -n "$PYTHON_CANDIDATES_TEMP$PYTHON_FAILURES_TEMP$PYTHON_SEEN_TEMP$PYTHON_UPDATE_TEMP$PYTHON_REPORT_TEMP" ]; then
        rm -f "$PYTHON_CANDIDATES_TEMP" "$PYTHON_FAILURES_TEMP" "$PYTHON_SEEN_TEMP" "$PYTHON_UPDATE_TEMP" "$PYTHON_REPORT_TEMP"
    fi
}
trap cleanup_setup_temp_files EXIT
trap 'exit 129' HUP
trap 'exit 130' INT
trap 'exit 143' TERM
run() {
    if [ "$DRY_RUN" = true ] || [ "$CHECK" = true ]; then
        echo "[dry-run] $*"
    else
        echo "+ $*"
        if [ "$1" = "$OPENBB_VENV/bin/python" ] && [ "$2" = -m ] && [ "$3" = pip ]; then PYTHONIOENCODING=utf-8 PYTHONUTF8=1 "$@"; else "$@"; fi
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
    PYTHONIOENCODING=utf-8 PYTHONUTF8=1 ARCHON_SETUP_PYTHON_SPECIFIER="$REQUIRES_PYTHON" "$1" "$SCRIPT_DIR/setup-trading-tools-python-probe.py" 2>&1
)
find_python() {
    candidates=$(mktemp "${TMPDIR:-/tmp}/setup-trading-tools-candidates.XXXXXX")
    seen_paths=$(mktemp "${TMPDIR:-/tmp}/setup-trading-tools-seen-paths.XXXXXX")
    update_temp=$(mktemp "${TMPDIR:-/tmp}/setup-trading-tools-candidate-update.XXXXXX")
    failures=$(mktemp "${TMPDIR:-/tmp}/setup-trading-tools-pip-errors.XXXXXX")
    PYTHON_CANDIDATES_TEMP=$candidates
    PYTHON_FAILURES_TEMP=$failures
    PYTHON_SEEN_TEMP=$seen_paths
    PYTHON_UPDATE_TEMP=$update_temp
    : > "$failures"
    specifier_satisfied=false
    selected_version=""
    selected_candidate=""
    selected_python_version=""
    saved_ifs=$IFS
    IFS=:
    set -f
    path_order=0
    for directory in $PATH; do
        [ -d "$directory" ] || continue
        set +f
        for candidate in "$directory"/python3 "$directory"/python "$directory"/python3.*; do
            [ -x "$candidate" ] || continue
            case "${candidate##*/}" in
                python3) preference=0; versioned=0 ;;
                python) preference=1; versioned=0 ;;
                python3.*) preference=2; versioned=${candidate##*.} ;;
            esac
            case "$versioned" in *[!0-9]*|'') versioned=0 ;; esac
            resolved_path=$candidate
            while [ -L "$resolved_path" ]; do
                link_target=$(readlink "$resolved_path" 2>/dev/null) || break
                case "$link_target" in
                    /*) resolved_path=$link_target ;;
                    *) resolved_path="${resolved_path%/*}/$link_target" ;;
                esac
            done
            resolved_directory=$(CDPATH= cd "${resolved_path%/*}" 2>/dev/null && pwd -P) || continue
            real_path="$resolved_directory/${resolved_path##*/}"
            if grep -F -x "$real_path" "$seen_paths" >/dev/null 2>&1; then
              if awk -F '\t' -v path="$real_path" '$8 == path { found=1 } END { exit !found }' "$candidates"; then
                old_fields=$(awk -F '\t' -v path="$real_path" '$8 == path { print $4 "\t" $5; exit }' "$candidates")
                old_preference=${old_fields%%"$(printf '\t')"*}
                old_versioned=${old_fields#*"$(printf '\t')"}
                if [ "$preference" -lt "$old_preference" ] || { [ "$preference" -eq "$old_preference" ] && [ "$versioned" -gt "$old_versioned" ]; }; then
                    awk -F '\t' -v path="$real_path" -v candidate="$candidate" -v preference="$preference" -v versioned="$versioned" 'BEGIN { OFS="\t" } $8 == path { $4=preference; $5=versioned; $7=candidate } { print }' "$candidates" > "$update_temp"
                    mv "$update_temp" "$candidates"
                    update_temp=$(mktemp "${TMPDIR:-/tmp}/setup-trading-tools-candidate-update.XXXXXX")
                    PYTHON_UPDATE_TEMP=$update_temp
                fi
              fi
              continue
            fi
            printf '%s\n' "$real_path" >> "$seen_paths"
            version=$("$candidate" -c 'import sys; print("%d.%d.%d" % sys.version_info[:3])' </dev/null 2>/dev/null) || continue
            case "$version" in *[!0-9.]*|'') continue ;; esac
            major=${version%%.*}
            remainder=${version#*.}
            minor=${remainder%%.*}
            patch=${remainder#*.}
            case "$patch" in *.*) patch=${patch%%.*} ;; esac
            printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
                "$major" "$minor" "$patch" "$preference" "$versioned" "$path_order" "$candidate" "$real_path" >> "$candidates"
        done
        set -f
        path_order=$((path_order + 1))
    done
    set +f
    IFS=$saved_ifs
    sort -t "$(printf '\t')" -k1,1nr -k2,2nr -k3,3nr -k4,4n -k5,5nr -k6,6n "$candidates" -o "$candidates"
    while IFS="$(printf '\t')" read -r major minor patch preference versioned path_order candidate real_path; do
        [ -n "$candidate" ] || continue
        if ! spec_error=$(python_satisfies_specifier "$candidate" </dev/null 2>&1); then
            printf 'OpenBB wheel probe: %s skipped; Python does not satisfy Requires-Python %s%s\n' "$candidate" "$REQUIRES_PYTHON" "${spec_error:+: $spec_error}"
            if [ -n "$spec_error" ]; then printf '%s: %s\n' "$candidate" "$spec_error" >> "$failures"; fi
            continue
        fi
        specifier_satisfied=true
        prepare_venv_rebuild || return 1
        printf '+ %s -m venv --clear %s\n' "$candidate" "$OPENBB_VENV" >&2
        if output=$("$candidate" -m venv --clear "$OPENBB_VENV" </dev/null 2>&1); then
            if [ ! -x "$OPENBB_VENV/bin/python" ]; then
                printf 'OpenBB wheel probe: %s pip error; venv creation did not create %s/bin/python\n' "$candidate" "$OPENBB_VENV"
                printf "setup-trading-tools.sh: venv creation with base interpreter '%s' did not create expected interpreter '%s/bin/python' in venv '%s'.\n%s\n" \
                    "${candidate##*/}" "$OPENBB_VENV" "$OPENBB_VENV" "$output" >> "$failures"
                continue
            fi
        else
            printf 'OpenBB wheel probe: %s pip error during venv creation\n%s\n' "$candidate" "$output"
            printf '%s venv creation:\n%s\n' "$candidate" "$output" >> "$failures"
            continue
        fi
        if output=$(PYTHONIOENCODING=utf-8 PYTHONUTF8=1 "$OPENBB_VENV/bin/python" -m pip install --upgrade pip </dev/null 2>&1); then
            :
        else
            printf 'OpenBB wheel probe: %s pip error during pip upgrade\n%s\n' "$candidate" "$output"
            printf '%s pip upgrade:\n%s\n' "$candidate" "$output" >> "$failures"
            continue
        fi
        PYTHON_REPORT_TEMP=$(mktemp "${TMPDIR:-/tmp}/setup-trading-tools-report.XXXXXX")
        if output=$(PYTHONIOENCODING=utf-8 PYTHONUTF8=1 "$OPENBB_VENV/bin/python" -m pip install --dry-run --only-binary=:all: --quiet --report "$PYTHON_REPORT_TEMP" --ignore-installed openbb openbb-platform-api </dev/null 2>&1); then
            resolved=$(PYTHONIOENCODING=utf-8 PYTHONUTF8=1 "$OPENBB_VENV/bin/python" -c 'import json,sys; report=json.load(open(sys.argv[1],encoding="utf-8")); print(next(item["metadata"]["version"] for item in report["install"] if item["metadata"]["name"].lower()=="openbb"))' "$PYTHON_REPORT_TEMP" 2>/dev/null) || resolved=""
            rm -f "$PYTHON_REPORT_TEMP"; PYTHON_REPORT_TEMP=""
            if [ -z "$resolved" ]; then
                printf 'OpenBB wheel probe: %s pip error; report did not contain openbb metadata/version\n' "$candidate"
                printf '%s wheel probe: successful pip exit but report did not contain openbb metadata/version\n%s\n' "$candidate" "$output" >> "$failures"
                continue
            fi
            printf 'OpenBB wheel probe: %s resolves openbb %s\n' "$candidate" "$resolved"
            if [ -z "$selected_version" ]; then higher=1; else
                if higher=$(PYTHONIOENCODING=utf-8 PYTHONUTF8=1 ARCHON_SETUP_PYTHON_COMPARE=1 "$candidate" "$SCRIPT_DIR/setup-trading-tools-python-probe.py" --compare "$resolved" "$selected_version" </dev/null 2>&1); then
                    case "$higher" in 1|0) ;; *) echo "setup-trading-tools.sh: could not compare OpenBB versions '$resolved' and '$selected_version': $higher" >&2; return 1 ;; esac
                else echo "setup-trading-tools.sh: could not compare OpenBB versions '$resolved' and '$selected_version': $higher" >&2; return 1; fi
            fi
            if [ "$higher" = 1 ]; then
                selected_version=$resolved
                selected_candidate=$candidate
                selected_python_version=$version
            fi
            continue
        fi
        rm -f "$PYTHON_REPORT_TEMP"; PYTHON_REPORT_TEMP=""
        printf 'OpenBB wheel probe: %s pip error\n%s\n' "$candidate" "$output"
        printf '%s wheel probe:\n%s\n' "$candidate" "$output" >> "$failures"
    done < "$candidates"
    if [ "$specifier_satisfied" != true ]; then
        echo "setup-trading-tools.sh: no Python interpreter satisfies OpenBB Requires-Python specifier '$REQUIRES_PYTHON'. $(cat "$failures")" >&2
        return 1
    fi
    if [ -z "$selected_candidate" ]; then
        echo "setup-trading-tools.sh: no Python interpreter can resolve OpenBB entirely from wheels. $(cat "$failures")" >&2
        return 1
    fi
    prepare_venv_rebuild || return 1
    printf '+ %s -m venv --clear %s (selected OpenBB resolver)\n' "$selected_candidate" "$OPENBB_VENV" >&2
    if output=$("$selected_candidate" -m venv --clear "$OPENBB_VENV" </dev/null 2>&1) && [ -x "$OPENBB_VENV/bin/python" ]; then
        if output=$(PYTHONIOENCODING=utf-8 PYTHONUTF8=1 "$OPENBB_VENV/bin/python" -m pip install --upgrade pip </dev/null 2>&1); then :; else
            printf '%s selected interpreter pip upgrade:\n%s\n' "$selected_candidate" "$output" >> "$failures"
            echo "setup-trading-tools.sh: selected Python could not prepare OpenBB install. $(cat "$failures")" >&2
            return 1
        fi
    else
        printf '%s selected interpreter venv creation:\n%s\n' "$selected_candidate" "$output" >> "$failures"
        echo "setup-trading-tools.sh: selected Python could not prepare OpenBB install. $(cat "$failures")" >&2
        return 1
    fi
    PYTHON_BASE=$selected_candidate
    PYTHON_BASE_VERSION=$selected_python_version
    OPENBB_RESOLVED_VERSION=$selected_version
    OPENBB_VENV_READY=true
    printf 'OpenBB Python: %s (Python %s) chosen because it resolves the highest OpenBB version, %s\n' "$PYTHON_BASE" "$PYTHON_BASE_VERSION" "$OPENBB_RESOLVED_VERSION"
    return 0
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
openbb_state() {
    if [ ! -d "$OPENBB_VENV" ]; then
        printf 'missing (%s)' "$OPENBB_VENV"
        return
    fi
    if [ ! -x "$OPENBB_VENV/bin/python" ]; then
        printf 'broken (missing interpreter: %s)' "$OPENBB_VENV/bin/python"
        return
    fi
    if output=$("$OPENBB_VENV/bin/python" -c 'import openbb' 2>&1); then
        printf 'healthy (%s)' "$OPENBB_VENV/bin/python"
    else
        probe_exit=$?
        printf 'broken (%s; import exit %s; output):\n%s' \
            "$OPENBB_VENV/bin/python" "$probe_exit" "$output"
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
    printf 'OpenBB venv: %s\n' "$(openbb_state)"
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
    (cd "$TV_DIR" && run npm install --omit=dev)
    chmod +x "$TV_DIR/scripts/"*.sh "$TV_DIR/src/cli/index.js" 2>/dev/null || true
fi

if [ "$SKIP_OPENBB" != true ]; then
    openbb_status=$(openbb_state)
    case "$openbb_status" in
      healthy*)
        :
        ;;
      *)
        prepare_venv_rebuild
        load_openbb_specifier
        find_python
        python_version=$("$PYTHON_BASE" --version </dev/null 2>&1)
        echo "OpenBB Python: ${PYTHON_BASE##*/} ($python_version)"
        if [ ! -f "$OPENBB_VENV/bin/python" ]; then
            echo "setup-trading-tools.sh: venv creation with base interpreter '$PYTHON_BASE' did not create expected interpreter '$OPENBB_VENV/bin/python' in venv '$OPENBB_VENV'." >&2
            exit 1
        fi
        ;;
    esac
    if [ "$OPENBB_VENV_READY" != true ]; then
        run "$OPENBB_VENV/bin/python" -m pip install --upgrade pip </dev/null
    fi
    run "$OPENBB_VENV/bin/python" -m pip install --only-binary=:all: openbb openbb-platform-api </dev/null
    if [ -n "$OPENBB_RESOLVED_VERSION" ]; then
        installed_version=""
        if installed_version=$("$OPENBB_VENV/bin/python" -c 'import importlib.metadata as m; print(m.version("openbb"))' 2>&1); then :; else
            installed_version="(unavailable: $installed_version)"
        fi
        if [ "$installed_version" != "$OPENBB_RESOLVED_VERSION" ]; then
            printf 'setup-trading-tools.sh: installed OpenBB version %s differs from wheel probe version %s\n' "$installed_version" "$OPENBB_RESOLVED_VERSION" >&2
            exit 1
        fi
    fi
    if [ -x "$OPENBB_VENV/bin/openbb-build" ]; then
        run "$OPENBB_VENV/bin/openbb-build"
    fi
    openbb_status=$(openbb_state)
    case "$openbb_status" in
      healthy*) : ;;
      *)
        printf 'setup-trading-tools.sh: OpenBB environment is still broken after installation: %s\n%s\n' \
            "$OPENBB_VENV" "$openbb_status" >&2
        exit 1
        ;;
    esac
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
