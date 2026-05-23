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
mkdir -p "$ARCHON_DIR/video-artifacts/downloads"
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

[policy.web]
allow_mutating_actions = false
allow_file_uploads = false
allow_pipeline_controls = false
allow_model_training_actions = false
allow_corpus_open_paths = false

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
rpm_limit = 12

[policy.docs.vlm.anthropic]
model = "claude-sonnet-4-6"

[policy.docs.vlm.openai_compat]
endpoint = "http://localhost:1234/v1"
model = "google/gemma-3-12b-it"
api_key_env = "OPENAI_API_KEY"
timeout_secs = 120
# Thinking models (Qwen3.6, GLM-4.5, etc.) emit reasoning into a separate
# field but still count those tokens against this cap. With max_tokens too
# low the model burns the entire budget on reasoning before producing
# answer content, and archon surfaces "chat completions response did not
# contain text". 8192 leaves comfortable headroom for both thinking and
# non-thinking responses on common vision models. Bump higher (32k+) for
# very verbose thinking models like Qwen3.6 q3_K_XL.
max_tokens = 8192
temperature = 0.2

[policy.docs.pdf]
extract_embedded_images = true
min_image_dimension = 200
min_image_bytes = 4096
vlm_per_page_image = true
render_text_pdf_pages = false

[policy.docs.retrieval]
exact_weight = 0.45
semantic_weight = 0.55

[policy.video]
enabled = false
allow_youtube = false
allow_direct_urls = false
allow_external_downloaders = false
allow_browser_automation = false
allow_caption_capture = false
allow_cloud_asr = false
allow_cloud_vlm = false
require_user_confirmation_for_download = true
max_duration_minutes = 120
max_download_mb = 2048
max_frames = 500
frame_interval_secs = 10
scene_change_threshold = 0.35
dedupe_threshold = 0.94

[policy.video.acquire]
browser_profile = "default"
external_downloader_bin = "yt-dlp"
po_token_provider = ""

[policy.video.asr]
provider = "whisper-rs"
model = "base"
device = "auto"
vad_stable_timestamps = false
model_cache_dir = ""
model_source = ""
diarization = false

[policy.video.frames]
mode = "scene"
ocr = true
vlm = true

[policy.video.summary]
enabled = false
allow_llm_summary = false
allow_cloud_summary = false
provider = "disabled"
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
echo "  .archon/video-artifacts/downloads/"
echo "  .archon/policy.toml"
if [ "$NO_AGENTS" = false ]; then
    echo "  .archon/agents/"
fi
echo "  prds/"
echo "  tasks/"

# ---------------------------------------------------------------------------
# Helpful hint about the system-deps installer (separate concern from this
# per-project init — this script never touches system packages or sudo).
# Verifies build, PDF/OCR, and video helper binaries are available; if any are
# missing, prints the one-liner to install them.
# ---------------------------------------------------------------------------
SCRIPT_DIR=$(dirname "$0")
DEPS_SCRIPT="$SCRIPT_DIR/install-system-deps.sh"
if [ -x "$DEPS_SCRIPT" ]; then
    if ! "$DEPS_SCRIPT" --check >/dev/null 2>&1; then
        echo
        echo "archon-init: system packages missing (build/PDF/OCR/video helper deps)."
        echo "             To install them: sudo $DEPS_SCRIPT"
        echo "             macOS/Homebrew users should omit sudo."
        echo "             To check: $DEPS_SCRIPT --check"
    fi
fi
