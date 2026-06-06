#!/bin/sh
# install-system-deps.sh — POSIX-compatible system-package installer for archon-cli.
#
# Detects the host OS and installs build deps, poppler PDF utilities
# (`pdftotext`, `pdfimages`, `pdftoppm`), Tesseract OCR, video ingest
# helpers (`ffmpeg`, `ffprobe`, `yt-dlp`, and `whisper-cli` where packaged),
# and optional Docker/OpenShell sandbox runtime dependencies.
#
# Does NOT install Rust, Python RapidOCR/OpenCV packages, VLM/Whisper model
# files, cloud OCR keys, provider credentials, or enable sandbox backends in
# config.toml. OpenShell gateway setup is opt-in.
#
# Usage:
#   sudo scripts/install-system-deps.sh         # install everything
#   scripts/install-system-deps.sh --dry-run    # show what would run, no changes
#   scripts/install-system-deps.sh --check      # verify deps already installed, no changes
#   sudo scripts/install-system-deps.sh --with-docker
#   sudo scripts/install-system-deps.sh --with-openshell
#   sudo scripts/install-system-deps.sh --with-sandbox   # Docker + OpenShell
#   sudo scripts/install-system-deps.sh --with-trading-tools
#   scripts/install-system-deps.sh --with-openshell --setup-openshell-gateway
#
# OpenShell extras follow NVIDIA's current support matrix: Debian/Ubuntu Linux
# x86_64/aarch64, WSL2 Debian/Ubuntu x86_64, and macOS Apple Silicon.
#
# Exit codes:
#   0   success (or all deps already present in --check mode)
#   1   usage / unknown OS
#   2   missing dependency (in --check mode)
#   3   package manager command failed
#
# Supports apt, dnf, pacman, zypper, apk, and macOS brew (pre-installed).

set -eu

SCRIPT_DIR=$(CDPATH= cd "$(dirname "$0")" && pwd)

DRY_RUN=false
CHECK_ONLY=false
WITH_DOCKER=false
WITH_OPENSHELL=false
SETUP_OPENSHELL_GATEWAY=false
WITH_TRADING_TOOLS=false

while [ $# -gt 0 ]; do
    case "$1" in
        --dry-run)                  DRY_RUN=true ;;
        --check)                    CHECK_ONLY=true ;;
        --with-docker)              WITH_DOCKER=true ;;
        --with-openshell)           WITH_OPENSHELL=true ;;
        --with-trading-tools)       WITH_TRADING_TOOLS=true ;;
        --setup-openshell-gateway|--start-openshell-gateway)
            WITH_OPENSHELL=true
            SETUP_OPENSHELL_GATEWAY=true
            ;;
        --with-sandbox)
            WITH_DOCKER=true
            WITH_OPENSHELL=true
            ;;
        --help|-h)
            awk '
                /^# Usage:/ { show = 1 }
                show && /^#/ { sub(/^# ?/, ""); print; next }
                show && !/^#/ { exit }
            ' "$0"
            exit 0
            ;;
        *)
            echo "install-system-deps.sh: unknown flag: $1" >&2
            exit 1
            ;;
    esac
    shift
done

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

if [ "$OS_FAMILY" = "macos" ]; then
    for brew_dir in /opt/homebrew/bin /usr/local/bin; do
        [ -d "$brew_dir" ] && PATH="$brew_dir:$PATH"
    done
    export PATH
fi

# Each variable below is a SPACE-SEPARATED list of package names appropriate
# for the selected package manager. The runner concatenates all three groups
# and runs them in a single pass for efficiency.
PKG_BUILD=""
PKG_PDF=""
PKG_OCR=""
PKG_VIDEO=""
PKG_DOCKER=""
PKG_OPENSHELL_PREREQ=""
PKG_TRADING_TOOLS=""
CHECK_WHISPER_CLI=false
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
        PKG_VIDEO="ffmpeg yt-dlp"
        PKG_DOCKER="docker.io"
        PKG_OPENSHELL_PREREQ="curl"
        PKG_TRADING_TOOLS="nodejs npm python3 python3-venv"
        ;;
    fedora|rhel|rocky|almalinux|centos)
        PKG_MGR="dnf"
        PKG_UPDATE_CMD=""   # dnf install handles refresh on demand
        PKG_INSTALL_CMD="dnf install -y"
        PKG_BUILD="gcc pkg-config openssl-devel git"
        PKG_PDF="poppler-utils"
        PKG_OCR="tesseract"
        PKG_VIDEO="ffmpeg-free yt-dlp"
        PKG_DOCKER="moby-engine docker-cli"
        PKG_OPENSHELL_PREREQ="curl"
        PKG_TRADING_TOOLS="nodejs npm python3"
        ;;
    arch|manjaro|endeavouros|garuda)
        PKG_MGR="pacman"
        PKG_UPDATE_CMD="pacman -Sy"
        PKG_INSTALL_CMD="pacman -S --needed --noconfirm"
        PKG_BUILD="base-devel openssl pkg-config git"
        PKG_PDF="poppler"
        PKG_OCR="tesseract"
        PKG_VIDEO="ffmpeg yt-dlp whisper.cpp"
        CHECK_WHISPER_CLI=true
        PKG_DOCKER="docker"
        PKG_OPENSHELL_PREREQ="curl"
        PKG_TRADING_TOOLS="nodejs npm python python-virtualenv"
        ;;
    opensuse-tumbleweed|opensuse-leap|opensuse|sles|sled)
        # OpenSUSE / SLE family. The poppler CLI utilities ship under
        # `poppler-tools` (note: NOT `poppler-utils` like Debian/Fedora).
        # `tesseract-ocr` is the language-pack-less core; for non-English
        # OCR users will need `tesseract-ocr-traineddata-<lang>` separately.
        PKG_MGR="zypper"
        PKG_UPDATE_CMD="zypper refresh"
        PKG_INSTALL_CMD="zypper install -y"
        PKG_BUILD="gcc pkg-config libopenssl-devel git"
        PKG_PDF="poppler-tools"
        PKG_OCR="tesseract-ocr"
        PKG_VIDEO="ffmpeg yt-dlp"
        PKG_DOCKER="docker"
        PKG_OPENSHELL_PREREQ="curl"
        PKG_TRADING_TOOLS="nodejs npm python3 python3-virtualenv"
        ;;
    alpine)
        # Alpine — common in containers. Note busybox `sh` already; the
        # script's POSIX-only constructs are fine. `--no-cache` skips
        # local index caching which is the standard apk convention.
        PKG_MGR="apk"
        PKG_UPDATE_CMD=""   # apk add --no-cache pulls fresh index per call
        PKG_INSTALL_CMD="apk add --no-cache"
        PKG_BUILD="build-base openssl-dev pkgconfig git"
        PKG_PDF="poppler-utils"
        PKG_OCR="tesseract-ocr"
        PKG_VIDEO="ffmpeg yt-dlp"
        PKG_DOCKER="docker"
        PKG_OPENSHELL_PREREQ="curl"
        PKG_TRADING_TOOLS="nodejs npm python3 py3-virtualenv"
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
        PKG_VIDEO="ffmpeg yt-dlp whisper-cpp"
        CHECK_WHISPER_CLI=true
        PKG_DOCKER=""
        PKG_OPENSHELL_PREREQ=""
        PKG_TRADING_TOOLS="node python"
        ;;
    *)
        echo "install-system-deps.sh: unsupported OS (uname=$UNAME_S, distro=$DISTRO_ID)" >&2
        echo "  Supported: ubuntu/debian/wsl2, fedora/rhel/rocky/centos/almalinux, arch/manjaro, opensuse/sles, alpine, macos" >&2
        echo "  Install manually:" >&2
        echo "    Build deps:        gcc/clang, pkg-config, openssl headers, git" >&2
        echo "    PDF utilities:     pdftotext + pdfimages + pdftoppm (poppler-utils)" >&2
        echo "    Image OCR:         tesseract-ocr" >&2
        echo "    Video ingest:      ffmpeg + ffprobe, yt-dlp, whisper.cpp/whisper-cli" >&2
        echo "    Sandbox extras:    docker CLI/engine and openshell CLI (optional)" >&2
        exit 1
        ;;
esac

if [ "$WITH_OPENSHELL" = true ]; then
    # NVIDIA OpenShell's local gateway path expects Docker to be available.
    # Remote-only gateway users can install just the `openshell` binary manually,
    # but the bundled installer chooses the safer local-ready setup.
    WITH_DOCKER=true
fi

HOST_ARCH=$(uname -m 2>/dev/null || echo unknown)
case "$HOST_ARCH" in
    arm64) HOST_ARCH="aarch64" ;;
    amd64) HOST_ARCH="x86_64" ;;
esac

if [ "$WITH_OPENSHELL" = true ]; then
    OPENSHELL_SUPPORTED=false
    case "$DISTRO_ID:$HOST_ARCH" in
        ubuntu:x86_64|ubuntu:aarch64|debian:x86_64|debian:aarch64|macos:aarch64)
            OPENSHELL_SUPPORTED=true
            ;;
    esac
    if [ "$OPENSHELL_SUPPORTED" != true ]; then
        echo "install-system-deps.sh: OpenShell is not enabled by this installer on $DISTRO_ID/$HOST_ARCH" >&2
        echo "  Supported OpenShell hosts follow NVIDIA's current matrix:" >&2
        echo "    Debian/Ubuntu Linux x86_64/aarch64, WSL2 Debian/Ubuntu x86_64, macOS Apple Silicon" >&2
        echo "  For this host, install Docker sandbox deps with: sudo $0 --with-docker" >&2
        exit 1
    fi
fi

# ---------------------------------------------------------------------------
# --check: verify presence of binaries, exit 2 if any missing
# ---------------------------------------------------------------------------
if [ "$CHECK_ONLY" = true ]; then
    MISSING=""
    # v0.1.47 unified PDF pipeline needs all three poppler binaries:
    #   pdftotext  — text-layer extraction
    #   pdfimages  — embedded image extraction
    #   pdftoppm   — page-render fallback for scanned PDFs
    for bin in gcc cc pkg-config git pdftotext pdfimages pdftoppm tesseract ffmpeg ffprobe yt-dlp; do
        if ! command -v "$bin" >/dev/null 2>&1; then
            MISSING="$MISSING $bin"
        fi
    done
    if [ "$CHECK_WHISPER_CLI" = true ] && ! command -v whisper-cli >/dev/null 2>&1; then
        MISSING="$MISSING whisper-cli"
    fi
    if [ "$WITH_DOCKER" = true ] && ! command -v docker >/dev/null 2>&1; then
        MISSING="$MISSING docker"
    fi
    if [ "$WITH_OPENSHELL" = true ] && ! command -v openshell >/dev/null 2>&1; then
        MISSING="$MISSING openshell"
    fi
    if [ "$SETUP_OPENSHELL_GATEWAY" = true ] && ! openshell status >/dev/null 2>&1; then
        MISSING="$MISSING openshell-gateway"
    fi
    if [ "$WITH_TRADING_TOOLS" = true ]; then
        for bin in node npm python3; do
            if ! command -v "$bin" >/dev/null 2>&1; then
                MISSING="$MISSING $bin"
            fi
        done
    fi
    # gcc OR cc satisfies the C compiler requirement
    if ! command -v gcc >/dev/null 2>&1 && ! command -v cc >/dev/null 2>&1; then
        :  # already in MISSING
    else
        MISSING=$(echo "$MISSING" | sed 's/ gcc//; s/ cc//')
    fi
    if [ -n "$MISSING" ]; then
        echo "install-system-deps.sh: missing:$MISSING" >&2
        if [ "$SETUP_OPENSHELL_GATEWAY" = true ]; then
            echo "  Run: $0 --with-openshell --setup-openshell-gateway" >&2
        elif [ "$WITH_OPENSHELL" = true ]; then
            echo "  Run: $0 --with-openshell" >&2
        elif [ "$WITH_DOCKER" = true ]; then
            [ "$PKG_MGR" = "brew" ] && echo "  Run: $0 --with-docker" >&2 || echo "  Run: sudo $0 --with-docker" >&2
        else
            [ "$PKG_MGR" = "brew" ] && echo "  Run: $0" >&2 || echo "  Run: sudo $0" >&2
        fi
        exit 2
    fi
    PRESENT="gcc/cc, pkg-config, git, pdftotext, pdfimages, pdftoppm, tesseract, ffmpeg, ffprobe, yt-dlp"
    if [ "$CHECK_WHISPER_CLI" = true ]; then
        PRESENT="$PRESENT, whisper-cli"
    fi
    if [ "$WITH_DOCKER" = true ]; then
        PRESENT="$PRESENT, docker"
    fi
    if [ "$WITH_OPENSHELL" = true ]; then
        PRESENT="$PRESENT, openshell"
    fi
    if [ "$SETUP_OPENSHELL_GATEWAY" = true ]; then
        PRESENT="$PRESENT, openshell-gateway"
    fi
    if [ "$WITH_TRADING_TOOLS" = true ]; then
        PRESENT="$PRESENT, node, npm, python3"
    fi
    echo "install-system-deps.sh: all requested binaries present ($PRESENT)"
    exit 0
fi

# ---------------------------------------------------------------------------
# Sudo handling — apt/dnf/pacman need root; brew must NOT run as root
# ---------------------------------------------------------------------------
SUDO=""
if [ "$PKG_MGR" != "brew" ]; then
    if [ "$(id -u 2>/dev/null || echo 1)" -ne 0 ]; then
        if [ "$DRY_RUN" = true ]; then
            SUDO="sudo"
        elif command -v sudo >/dev/null 2>&1; then
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
ALL_PKGS="$PKG_BUILD $PKG_PDF $PKG_OCR $PKG_VIDEO"
if [ "$WITH_DOCKER" = true ]; then
    ALL_PKGS="$ALL_PKGS $PKG_DOCKER"
fi
if [ "$WITH_OPENSHELL" = true ]; then
    ALL_PKGS="$ALL_PKGS $PKG_OPENSHELL_PREREQ"
fi
if [ "$WITH_TRADING_TOOLS" = true ]; then
    ALL_PKGS="$ALL_PKGS $PKG_TRADING_TOOLS"
fi
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
echo "install-system-deps.sh: sandbox extras: docker=$WITH_DOCKER openshell=$WITH_OPENSHELL"
echo "install-system-deps.sh: trading tools deps: $WITH_TRADING_TOOLS"
if [ "$SETUP_OPENSHELL_GATEWAY" = true ]; then
    echo "install-system-deps.sh: OpenShell gateway setup requested"
fi
if [ "$PKG_MGR" = "brew" ]; then
    if ! command -v brew >/dev/null 2>&1; then
        echo "install-system-deps.sh: Homebrew not found. Install from https://brew.sh first, then re-run." >&2
        exit 1
    fi
    echo "install-system-deps.sh: Note — install Xcode Command Line Tools separately if not yet present:"
    echo "    xcode-select --install"
fi

install_macos_docker() {
    if [ "$WITH_DOCKER" != true ] || [ "$PKG_MGR" != "brew" ]; then
        return 0
    fi
    if command -v docker >/dev/null 2>&1; then
        echo "install-system-deps.sh: docker already present"
        return 0
    fi
    if [ "$DRY_RUN" = true ]; then
        echo "[dry-run] brew install --cask docker"
        return 0
    fi
    echo "+ brew install --cask docker"
    brew install --cask docker || {
        echo "install-system-deps.sh: Docker Desktop install failed" >&2
        exit 3
    }
}

. "$SCRIPT_DIR/lib/openshell-setup.sh"

if [ -n "$PKG_UPDATE_CMD" ]; then
    # shellcheck disable=SC2086
    run $SUDO $PKG_UPDATE_CMD || {
        echo "install-system-deps.sh: package index update failed" >&2
        exit 3
    }
fi

# shellcheck disable=SC2086
if [ -n "$ALL_PKGS" ]; then
    # shellcheck disable=SC2086
    run $SUDO $PKG_INSTALL_CMD $ALL_PKGS || {
        echo "install-system-deps.sh: package install failed" >&2
        exit 3
    }
fi

install_macos_docker
install_openshell
setup_openshell_gateway

# ---------------------------------------------------------------------------
# Post-install verification
# ---------------------------------------------------------------------------
if [ "$DRY_RUN" = false ]; then
    echo
    echo "install-system-deps.sh: verifying installs..."
    VERIFY_BINS="pdftotext pdfimages pdftoppm tesseract ffmpeg ffprobe yt-dlp"
    if [ "$CHECK_WHISPER_CLI" = true ]; then
        VERIFY_BINS="$VERIFY_BINS whisper-cli"
    fi
    if [ "$WITH_DOCKER" = true ]; then
        VERIFY_BINS="$VERIFY_BINS docker"
    fi
    if [ "$WITH_OPENSHELL" = true ]; then
        VERIFY_BINS="$VERIFY_BINS openshell"
    fi
    if [ "$WITH_TRADING_TOOLS" = true ]; then
        VERIFY_BINS="$VERIFY_BINS node npm python3"
    fi
    for bin in $VERIFY_BINS; do
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
    echo "  4. For local video ASR, download a whisper.cpp model and set [policy.video.asr].model"
    echo "     Optional RapidOCR/OpenCV fallback: python3 -m pip install rapidocr opencv-python"
    echo "     Optional Trading Lab tools: scripts/setup-trading-tools.sh --target /path/to/project"
    if [ "$WITH_DOCKER" = true ]; then
        echo "  5. Enable Docker sandboxing by setting [sandbox].backend=\"docker\" and [sandbox.docker].enabled=true"
    fi
    if [ "$WITH_OPENSHELL" = true ]; then
        if [ "$SETUP_OPENSHELL_GATEWAY" = true ]; then
            echo "  5. Enable OpenShell sandboxing by setting [sandbox].backend=\"openshell\" and [sandbox.openshell].enabled=true"
            echo "  6. Test mirror mode from your project: openshell sandbox create --no-keep -- /bin/bash -lc \"cd -- '\\$PWD' && pwd && ls\""
        else
            echo "  5. Start/check the OpenShell gateway: $0 --with-openshell --setup-openshell-gateway"
            echo "  6. Enable OpenShell sandboxing by setting [sandbox].backend=\"openshell\" and [sandbox.openshell].enabled=true"
        fi
    fi
fi
