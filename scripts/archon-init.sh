#!/bin/sh
# archon-init.sh — POSIX-compatible project initialiser for archon-cli.
#
# Creates the .archon/ directory tree and starter files so a project is
# ready for archon-cli immediately. Always idempotent — safe to re-run.
#
# Usage:
#   archon-init.sh [--target <dir>] [--archon-cli-repo <path>] [--no-agents]
#
#   --target DIR         Project root (default: $PWD)
#   --archon-cli-repo PATH  Copy bundled skills/templates from a source tree
#   --no-agents          Skip .archon/agents/ directory creation
#
# Exit codes:
#   0   success
#   1   usage / invalid arguments
#   2   target directory is not writable

set -eu

# ---------------------------------------------------------------------------
# Defaults
# ---------------------------------------------------------------------------
TARGET="${PWD:-.}"
ARCHON_CLI_REPO=""
NO_AGENTS=false

# ---------------------------------------------------------------------------
# Parse arguments
# ---------------------------------------------------------------------------
while [ $# -gt 0 ]; do
    case "$1" in
        --target)
            shift
            TARGET="$1"
            ;;
        --archon-cli-repo)
            shift
            ARCHON_CLI_REPO="$1"
            ;;
        --no-agents)
            NO_AGENTS=true
            ;;
        --help|-h)
            echo "Usage: archon-init.sh [--target <dir>] [--archon-cli-repo <path>] [--no-agents]"
            exit 0
            ;;
        *)
            echo "archon-init.sh: unknown flag: $1" >&2
            echo "Usage: archon-init.sh [--target <dir>] [--archon-cli-repo <path>] [--no-agents]"
            exit 1
            ;;
    esac
    shift
done

# ---------------------------------------------------------------------------
# Validate target
# ---------------------------------------------------------------------------
if [ ! -d "$TARGET" ]; then
    echo "archon-init.sh: target is not a directory: $TARGET" >&2
    exit 2
fi

if [ ! -w "$TARGET" ]; then
    echo "archon-init.sh: target is not writable: $TARGET" >&2
    exit 2
fi

# ---------------------------------------------------------------------------
# Create directory tree
# ---------------------------------------------------------------------------
ARCHON_DIR="$TARGET/.archon"
mkdir -p "$ARCHON_DIR"
mkdir -p "$ARCHON_DIR/skills"
mkdir -p "$ARCHON_DIR/templates"
mkdir -p "$ARCHON_DIR/adr"
mkdir -p "$ARCHON_DIR/context"
mkdir -p "$ARCHON_DIR/specs"
mkdir -p "$ARCHON_DIR/docs"
mkdir -p "$ARCHON_DIR/docs/inbox"
mkdir -p "$ARCHON_DIR/docs/images"
mkdir -p "$ARCHON_DIR/evidence"
mkdir -p "$TARGET/prds"
mkdir -p "$TARGET/tasks"

if [ "$NO_AGENTS" = false ]; then
    mkdir -p "$ARCHON_DIR/agents"
fi

# ---------------------------------------------------------------------------
# Copy skills + templates from a source tree (optional)
# ---------------------------------------------------------------------------
if [ -n "$ARCHON_CLI_REPO" ]; then
    ASSETS_DIR="$ARCHON_CLI_REPO/assets"
    if [ -d "$ASSETS_DIR/skills" ]; then
        cp -r "$ASSETS_DIR/skills/." "$ARCHON_DIR/skills/" 2>/dev/null || true
    fi
    if [ -d "$ASSETS_DIR/templates" ]; then
        cp -r "$ASSETS_DIR/templates/." "$ARCHON_DIR/templates/" 2>/dev/null || true
    fi
    if [ -f "$ARCHON_CLI_REPO/.archon/specs/gametheory.yaml" ]; then
        cp "$ARCHON_CLI_REPO/.archon/specs/gametheory.yaml" "$ARCHON_DIR/specs/gametheory.yaml" 2>/dev/null || true
    fi
    if [ "$NO_AGENTS" = false ] && [ -d "$ARCHON_CLI_REPO/.archon/agents/gametheory" ]; then
        mkdir -p "$ARCHON_DIR/agents/gametheory"
        cp -r "$ARCHON_CLI_REPO/.archon/agents/gametheory/." "$ARCHON_DIR/agents/gametheory/" 2>/dev/null || true
    fi
fi

# ---------------------------------------------------------------------------
# Create default Evidence Engine policy if absent.
# ---------------------------------------------------------------------------
POLICY_FILE="$ARCHON_DIR/policy.toml"
if [ ! -f "$POLICY_FILE" ]; then
    cat > "$POLICY_FILE" <<'EOF'
# archon Evidence Engine policy. Safe defaults: local-first and default-deny
# for cloud/networked or auto-applying behaviour.

[policy.network]
default = "deny"
allow_cloud_vlm = false
allow_web_strategy_agents = false
allow_mcp_server_exposure = false

[policy.workers]
ocr = "allow-local"
embedding = "allow-local"
vlm = "deny"
web_fetch = "deny"

[policy.gametheory]
max_agents_per_council = 12
max_cost_usd = 20.00
enable_tier11 = false
allow_web_tools = false

[policy.learning]
auto_apply_low_risk = false
require_approval_for_prompt_changes = true
require_approval_for_blocking_gates = true
require_approval_for_network_changes = true

[policy.docs.vlm]
enabled = false
mode = "disabled"
provider = "disabled"
allow_cloud = false
require_user_confirmation_for_cloud = true

[policy.docs.vlm.ollama]
endpoint = "http://localhost:11434"
model = "gemma4:e4b"
timeout_secs = 120

[policy.docs.vlm.gemini]
api_key_env = "GOOGLE_API_KEY"
model = "gemini-3-flash-preview"
endpoint_base = "https://generativelanguage.googleapis.com/v1beta"
rpm_limit = 15

[policy.docs.vlm.anthropic]
model = "claude-sonnet-4-6"

[policy.docs.pdf]
extract_embedded_images = true
min_image_dimension = 200
min_image_bytes = 4096
vlm_per_page_image = true
render_text_pdf_pages = false

[policy.docs.retrieval]
exact_weight = 0.45
semantic_weight = 0.55
EOF
fi

# ---------------------------------------------------------------------------
# Create .gitignore if it doesn't already cover .archon
# ---------------------------------------------------------------------------
GITIGNORE="$TARGET/.gitignore"
if [ -f "$GITIGNORE" ]; then
    if ! grep -q '\.archon' "$GITIGNORE" 2>/dev/null; then
        printf '\n# archon-cli working directory\n.archon/\n' >> "$GITIGNORE"
    fi
else
    printf '# archon-cli working directory\n.archon/\n' > "$GITIGNORE"
fi

echo "archon-init: project initialised at $TARGET"
echo "  .archon/skills/"
echo "  .archon/templates/"
echo "  .archon/adr/"
echo "  .archon/context/"
echo "  .archon/specs/"
echo "  .archon/docs/inbox/"
echo "  .archon/docs/images/"
echo "  .archon/evidence/"
echo "  .archon/policy.toml"
if [ "$NO_AGENTS" = false ]; then
    echo "  .archon/agents/"
fi
echo "  prds/"
echo "  tasks/"

# ---------------------------------------------------------------------------
# Helpful hint about the system-deps installer (separate concern from this
# per-project init — this script never touches system packages or sudo).
# Verifies build-essential / pdftotext / tesseract are available; if any
# are missing, prints the one-liner to install them.
# ---------------------------------------------------------------------------
SCRIPT_DIR=$(dirname "$0")
DEPS_SCRIPT="$SCRIPT_DIR/install-system-deps.sh"
if [ -x "$DEPS_SCRIPT" ]; then
    if ! "$DEPS_SCRIPT" --check >/dev/null 2>&1; then
        echo
        echo "archon-init: system packages missing (build deps / pdftotext / tesseract)."
        echo "             To install them: sudo $DEPS_SCRIPT"
        echo "             To check: $DEPS_SCRIPT --check"
    fi
fi
