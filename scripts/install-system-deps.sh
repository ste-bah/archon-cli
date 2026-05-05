#!/bin/sh
# install-system-deps.sh — POSIX-compatible system-package installer for archon-cli.
#
# Detects the host OS and installs everything archon-cli needs to build
# and run end-to-end:
#
#   Build deps         build-essential / gcc + pkg-config + openssl headers + git
#   PDF text extraction pdftotext (poppler-utils) — required by `archon docs ingest` for native-text PDFs
#   Image OCR          tesseract-ocr — required by `archon docs ingest` for PNG/JPEG/TIFF + scanned PDF pages
#
# Does NOT install Rust — use rustup directly per docs/getting-started/installation.md.
# Does NOT install optional extras (VLM models, cloud OCR keys, etc).
#
# Usage:
#   sudo scripts/install-system-deps.sh         # install everything
#   scripts/install-system-deps.sh --dry-run    # show what would run, no changes
#   scripts/install-system-deps.sh --check      # verify deps already installed, no changes
#
# Exit codes:
#   0   success (or all deps already present in --check mode)
#   1   usage / unknown OS
#   2   missing dependency (in --check mode)
#   3   package manager command failed
#
# Supports: Ubuntu/Debian/WSL2 (apt), Fedora/RHEL/Rocky (dnf), Arch/Manjaro (pacman),
#           macOS (brew — must be pre-installed)

set -eu

# ---------------------------------------------------------------------------
# Args
# ---------------------------------------------------------------------------
DRY_RUN=false
CHECK_ONLY=false

while [ $# -gt 0 ]; do
    case "$1" in
        --dry-run) DRY_RUN=true ;;
        --check)   CHECK_ONLY=true ;;
        --help|-h)
            sed -n '/^# Usage:/,/^# Supports:/p' "$0" | sed 's/^# \{0,1\}//'
            exit 0
            ;;
        *)
            echo "install-system-deps.sh: unknown flag: $1" >&2
            exit 1
            ;;
    esac
    shift
done

# ---------------------------------------------------------------------------
# OS detection
# ---------------------------------------------------------------------------
UNAME_S="$(uname -s 2>/dev/null || echo unknown)"

OS_FAMILY="unknown"
DISTRO_ID="unknown"

case "$UNAME_S" in
    Linux)
        OS_FAMILY="linux"
        if [ -r /etc/os-release ]; then
            # shellcheck disable=SC1091
            . /etc/os-release
            DISTRO_ID="${ID:-unknown}"
        fi
        ;;
    Darwin)
        OS_FAMILY="macos"
        DISTRO_ID="macos"
        ;;
    *)
        OS_FAMILY="unknown"
        ;;
esac

# ---------------------------------------------------------------------------
# Per-OS package lists
# ---------------------------------------------------------------------------
# Each variable below is a SPACE-SEPARATED list of package names appropriate
# for the selected package manager. The runner concatenates all three groups
# and runs them in a single pass for efficiency.
PKG_BUILD=""
PKG_PDF=""
PKG_OCR=""
PKG_MGR=""
PKG_INSTALL_CMD=""
PKG_UPDATE_CMD=""

case "$DISTRO_ID" in
    ubuntu|debian|raspbian|linuxmint|pop|elementary)
        PKG_MGR="apt"
        PKG_UPDATE_CMD="apt-get update"
        PKG_INSTALL_CMD="apt-get install -y"
        PKG_BUILD="build-essential pkg-config libssl-dev git"
        PKG_PDF="poppler-utils"
        PKG_OCR="tesseract-ocr"
        ;;
    fedora|rhel|rocky|almalinux|centos)
        PKG_MGR="dnf"
        PKG_UPDATE_CMD=""   # dnf install handles refresh on demand
        PKG_INSTALL_CMD="dnf install -y"
        PKG_BUILD="gcc pkg-config openssl-devel git"
        PKG_PDF="poppler-utils"
        PKG_OCR="tesseract"
        ;;
    arch|manjaro|endeavouros|garuda)
        PKG_MGR="pacman"
        PKG_UPDATE_CMD="pacman -Sy"
        PKG_INSTALL_CMD="pacman -S --needed --noconfirm"
        PKG_BUILD="base-devel openssl pkg-config git"
        PKG_PDF="poppler"
        PKG_OCR="tesseract"
        ;;
    macos)
        PKG_MGR="brew"
        PKG_UPDATE_CMD="brew update"
        PKG_INSTALL_CMD="brew install"
        # Build deps come from Xcode Command Line Tools — installed separately
        # via `xcode-select --install` (no Homebrew formula).
        PKG_BUILD=""
        PKG_PDF="poppler"
        PKG_OCR="tesseract"
        ;;
    *)
        echo "install-system-deps.sh: unsupported OS (uname=$UNAME_S, distro=$DISTRO_ID)" >&2
        echo "  Supported: ubuntu/debian/wsl2, fedora/rhel/rocky/centos/almalinux, arch/manjaro, macos" >&2
        echo "  Install manually:" >&2
        echo "    Build deps:        gcc/clang, pkg-config, openssl headers, git" >&2
        echo "    PDF text:          pdftotext (poppler-utils)" >&2
        echo "    Image OCR:         tesseract-ocr" >&2
        exit 1
        ;;
esac

# ---------------------------------------------------------------------------
# --check: verify presence of binaries, exit 2 if any missing
# ---------------------------------------------------------------------------
if [ "$CHECK_ONLY" = true ]; then
    MISSING=""
    for bin in gcc cc pkg-config git pdftotext tesseract; do
        if ! command -v "$bin" >/dev/null 2>&1; then
            MISSING="$MISSING $bin"
        fi
    done
    # gcc OR cc satisfies the C compiler requirement
    if ! command -v gcc >/dev/null 2>&1 && ! command -v cc >/dev/null 2>&1; then
        :  # already in MISSING
    else
        MISSING=$(echo "$MISSING" | sed 's/ gcc//; s/ cc//')
    fi
    if [ -n "$MISSING" ]; then
        echo "install-system-deps.sh: missing:$MISSING" >&2
        echo "  Run: sudo $0" >&2
        exit 2
    fi
    echo "install-system-deps.sh: all required binaries present (gcc/cc, pkg-config, git, pdftotext, tesseract)"
    exit 0
fi

# ---------------------------------------------------------------------------
# Sudo handling — apt/dnf/pacman need root; brew must NOT run as root
# ---------------------------------------------------------------------------
SUDO=""
if [ "$PKG_MGR" != "brew" ]; then
    if [ "$(id -u 2>/dev/null || echo 1)" -ne 0 ]; then
        if command -v sudo >/dev/null 2>&1; then
            SUDO="sudo"
        else
            echo "install-system-deps.sh: must run as root (sudo not found)" >&2
            exit 1
        fi
    fi
else
    if [ "$(id -u 2>/dev/null || echo 1)" -eq 0 ]; then
        echo "install-system-deps.sh: do NOT run brew as root. Re-run as your normal user." >&2
        exit 1
    fi
fi

# ---------------------------------------------------------------------------
# Dry-run prints the commands; otherwise execute
# ---------------------------------------------------------------------------
ALL_PKGS="$PKG_BUILD $PKG_PDF $PKG_OCR"
# Trim leading space if PKG_BUILD was empty (macOS case)
ALL_PKGS=$(echo "$ALL_PKGS" | sed 's/^ *//')

run() {
    if [ "$DRY_RUN" = true ]; then
        echo "[dry-run] $*"
    else
        echo "+ $*"
        # shellcheck disable=SC2086
        $@
    fi
}

echo "install-system-deps.sh: detected $OS_FAMILY/$DISTRO_ID, package manager: $PKG_MGR"
if [ "$PKG_MGR" = "brew" ]; then
    if ! command -v brew >/dev/null 2>&1; then
        echo "install-system-deps.sh: Homebrew not found. Install from https://brew.sh first, then re-run." >&2
        exit 1
    fi
    echo "install-system-deps.sh: Note — install Xcode Command Line Tools separately if not yet present:"
    echo "    xcode-select --install"
fi

if [ -n "$PKG_UPDATE_CMD" ]; then
    # shellcheck disable=SC2086
    run $SUDO $PKG_UPDATE_CMD || {
        echo "install-system-deps.sh: package index update failed" >&2
        exit 3
    }
fi

# shellcheck disable=SC2086
run $SUDO $PKG_INSTALL_CMD $ALL_PKGS || {
    echo "install-system-deps.sh: package install failed" >&2
    exit 3
}

# ---------------------------------------------------------------------------
# Post-install verification
# ---------------------------------------------------------------------------
if [ "$DRY_RUN" = false ]; then
    echo
    echo "install-system-deps.sh: verifying installs..."
    for bin in pdftotext tesseract; do
        if command -v "$bin" >/dev/null 2>&1; then
            VERSION=$("$bin" --version 2>&1 | head -n 1 || echo "(version check failed)")
            echo "  ok: $bin     $VERSION"
        else
            echo "  MISSING: $bin (post-install check failed)" >&2
        fi
    done
    echo
    echo "install-system-deps.sh: done. Next steps:"
    echo "  1. Install Rust 1.85+ if not already: curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh"
    echo "  2. Build archon-cli: cargo build --release --bin archon"
    echo "  3. Initialise a project: ./scripts/archon-init.sh --target /path/to/project"
fi
