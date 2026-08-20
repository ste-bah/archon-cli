#!/bin/sh
# install-system-deps.sh — POSIX-compatible system-package installer for archon-cli.
#
# Detects the host OS and installs build deps, poppler PDF utilities
# (`pdftotext`, `pdfimages`, `pdftoppm`), Tesseract OCR, video ingest
# helpers (`ffmpeg`, `ffprobe`, `yt-dlp`, and `whisper-cli` where packaged),
# and optional Docker/OpenShell sandbox runtime dependencies.
#
# Also installs rustup (per-user, always as the *invoking* user — under sudo
# the installer runs as $SUDO_USER so Rust never lands in root's home) unless
# --no-rust is given. The repo pins its toolchain in rust-toolchain.toml;
# rustup fetches the pinned version on first build.
#
# Does NOT install Python RapidOCR/OpenCV packages, VLM/Whisper model
# files, cloud OCR keys, provider credentials, or enable sandbox backends in
# config.toml. OpenShell gateway setup is opt-in.
#
# Usage:
#   sudo scripts/install-system-deps.sh         # install everything (incl. rustup)
#   scripts/install-system-deps.sh --dry-run    # show what would run, no changes
#   scripts/install-system-deps.sh --check      # verify deps already installed, no changes
#   sudo scripts/install-system-deps.sh --no-rust   # skip the rustup install
#   sudo scripts/install-system-deps.sh --with-docker
#   sudo scripts/install-system-deps.sh --with-openshell
#   sudo scripts/install-system-deps.sh --with-sandbox   # Docker + OpenShell
#   sudo scripts/install-system-deps.sh --with-trading-tools
#   sudo scripts/install-system-deps.sh --with-ocr
#   sudo scripts/install-system-deps.sh --with-java   # JDK, Gradle, Maven
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
# Supports apt, dnf (Fedora/RHEL family and Amazon Linux 2023+), pacman,
# zypper, apk, and macOS brew (pre-installed).
#
# Amazon Linux 2023 note: tesseract, ffmpeg, yt-dlp, and whisper are not in
# the AL2023 repos. ffmpeg/ffprobe are installed from the static
# johnvansickle.com builds and yt-dlp from its official GitHub release
# binary; tesseract has no packaged fallback (use the RapidOCR Python
# fallback for OCR instead). Amazon Linux 2 reached end-of-life on
# 2026-06-30 and is rejected with manual instructions.

set -eu

SCRIPT_DIR=$(CDPATH= cd "$(dirname "$0")" && pwd)

DRY_RUN=false
CHECK_ONLY=false
WITH_DOCKER=false
WITH_OPENSHELL=false
SETUP_OPENSHELL_GATEWAY=false
WITH_TRADING_TOOLS=false
WITH_OCR=false
WITH_JAVA=false
WITH_RUST=true

while [ $# -gt 0 ]; do
    case "$1" in
        --dry-run)                  DRY_RUN=true ;;
        --check)                    CHECK_ONLY=true ;;
        --no-rust)                  WITH_RUST=false ;;
        --with-docker)              WITH_DOCKER=true ;;
        --with-openshell)           WITH_OPENSHELL=true ;;
        --with-trading-tools)       WITH_TRADING_TOOLS=true ;;
        --with-ocr)                 WITH_OCR=true ;;
        --with-java)                WITH_JAVA=true ;;
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

# ---------------------------------------------------------------------------
# Rust target user — rustup is per-user. Under sudo, install as the invoking
# user ($SUDO_USER), never root, so ~/.cargo lands in the right home.
# ---------------------------------------------------------------------------
RUST_USER=""
RUST_HOME="$HOME"
if [ "$(id -u 2>/dev/null || echo 1)" -eq 0 ] && [ -n "${SUDO_USER:-}" ] && [ "$SUDO_USER" != "root" ]; then
    RUST_USER="$SUDO_USER"
    RUST_HOME=$(getent passwd "$SUDO_USER" 2>/dev/null | cut -d: -f6)
    [ -n "$RUST_HOME" ] || RUST_HOME="/home/$SUDO_USER"
fi

have_cargo() {
    command -v cargo >/dev/null 2>&1 || [ -x "$RUST_HOME/.cargo/bin/cargo" ]
}

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
# Maven (and unzip) for --with-java. The JDK is resolved separately by
# resolve_jdk_package, and Gradle is deliberately absent on Linux — see
# install_gradle below.
PKG_JAVA=""
# OpenJDK packages to try, newest first. The first that the distribution
# actually carries wins.
JDK_CANDIDATES=""
# True where the package manager ships a Gradle new enough to be worth using.
GRADLE_FROM_PKG_MGR=false
# Homebrew cask for the macOS JDK. Pinned to the current LTS rather than the
# bare `temurin` cask: that tracks the newest six-month feature release, which
# is out of support as soon as its successor ships.
MACOS_JDK_CASK="temurin@25"
CHECK_WHISPER_CLI=false
# Set for distros whose repos do not package tesseract at all (Amazon Linux
# 2023). --check and post-install verification then treat tesseract as a
# warning rather than a failure, pointing at the RapidOCR fallback.
TESSERACT_UNPACKAGED=false
# Set for distros needing binary-download fallbacks for ffmpeg/yt-dlp.
AMZN_BINARY_FALLBACKS=false
PKG_MGR=""
PKG_INSTALL_CMD=""
PKG_UPDATE_CMD=""

case "$DISTRO_ID" in
    ubuntu|debian|raspbian|linuxmint|pop|elementary)
        PKG_MGR="apt"
        PKG_UPDATE_CMD="apt-get update"
        PKG_INSTALL_CMD="apt-get install -y"
        PKG_BUILD="build-essential pkg-config libssl-dev git curl libclang-dev perl cmake libasound2-dev"
        PKG_PDF="poppler-utils"
        PKG_OCR="tesseract-ocr"
        PKG_VIDEO="ffmpeg yt-dlp"
        PKG_DOCKER="docker.io"
        PKG_OPENSHELL_PREREQ="curl"
        PKG_TRADING_TOOLS="nodejs npm python3 python3-venv"
        PKG_JAVA="maven unzip"
        # `default-jdk` is not used: it tracks the release's default, which is
        # frequently several LTS versions behind (Ubuntu 24.04 still defaults
        # to 21). It remains the last candidate so there is always something.
        JDK_CANDIDATES="openjdk-25-jdk openjdk-21-jdk default-jdk"
        ;;
    fedora|rhel|rocky|almalinux|centos)
        PKG_MGR="dnf"
        PKG_UPDATE_CMD=""   # dnf install handles refresh on demand
        PKG_INSTALL_CMD="dnf install -y"
        PKG_BUILD="gcc pkg-config openssl-devel git clang perl make cmake alsa-lib-devel"
        PKG_PDF="poppler-utils"
        PKG_OCR="tesseract"
        PKG_VIDEO="ffmpeg-free yt-dlp"
        PKG_DOCKER="moby-engine docker-cli"
        PKG_OPENSHELL_PREREQ="curl"
        PKG_TRADING_TOOLS="nodejs npm python3"
        PKG_JAVA="maven unzip"
        JDK_CANDIDATES="java-25-openjdk-devel java-21-openjdk-devel"
        ;;
    amzn)
        # Amazon Linux. AL2023+ uses dnf against a deliberately small core
        # repo: tesseract, ffmpeg, yt-dlp, and whisper are NOT packaged.
        # ffmpeg + yt-dlp are installed from upstream binaries below
        # (install_amzn_extras); tesseract has no packaged fallback.
        # `pkg-config` is provided by `pkgconf-pkg-config` on AL2023.
        # tar + xz are needed to unpack the static ffmpeg build.
        if [ "${VERSION_ID:-unknown}" = "2" ]; then
            echo "install-system-deps.sh: Amazon Linux 2 reached end-of-life on 2026-06-30 and is not supported." >&2
            echo "  Upgrade to Amazon Linux 2023, or install manually:" >&2
            echo "    sudo yum install -y gcc pkgconfig openssl-devel git poppler-utils" >&2
            echo "    tesseract via EPEL; ffmpeg/yt-dlp via static builds (see AL2023 notes in this script)" >&2
            exit 1
        fi
        PKG_MGR="dnf"
        PKG_UPDATE_CMD=""   # dnf install handles refresh on demand
        PKG_INSTALL_CMD="dnf install -y"
        PKG_BUILD="gcc pkgconf-pkg-config openssl-devel git tar xz clang perl make cmake alsa-lib-devel"
        PKG_PDF="poppler-utils"
        PKG_OCR=""          # not packaged on AL2023 — see TESSERACT_UNPACKAGED
        PKG_VIDEO=""        # ffmpeg/yt-dlp via install_amzn_extras
        TESSERACT_UNPACKAGED=true
        AMZN_BINARY_FALLBACKS=true
        PKG_DOCKER="docker"
        PKG_OPENSHELL_PREREQ="curl"
        PKG_TRADING_TOOLS="nodejs npm python3"
        PKG_JAVA="maven unzip"
        # Corretto is Amazon's OpenJDK build; it is what AL2023 packages.
        JDK_CANDIDATES="java-25-amazon-corretto-devel java-21-amazon-corretto-devel"
        ;;
    arch|manjaro|endeavouros|garuda)
        PKG_MGR="pacman"
        PKG_UPDATE_CMD="pacman -Sy"
        PKG_INSTALL_CMD="pacman -S --needed --noconfirm"
        PKG_BUILD="base-devel openssl pkg-config git curl clang perl cmake alsa-lib"
        PKG_PDF="poppler"
        PKG_OCR="tesseract"
        PKG_VIDEO="ffmpeg yt-dlp whisper.cpp"
        CHECK_WHISPER_CLI=true
        PKG_DOCKER="docker"
        PKG_OPENSHELL_PREREQ="curl"
        PKG_TRADING_TOOLS="nodejs npm python python-virtualenv"
        # Arch tracks Gradle upstream closely, so its package is worth using.
        # Arch's `jdk-openjdk` always tracks the newest release, so there is
        # nothing to probe for.
        PKG_JAVA="jdk-openjdk maven gradle unzip"
        GRADLE_FROM_PKG_MGR=true
        ;;
    opensuse-tumbleweed|opensuse-leap|opensuse|sles|sled)
        # OpenSUSE / SLE family. The poppler CLI utilities ship under
        # `poppler-tools` (note: NOT `poppler-utils` like Debian/Fedora).
        # `tesseract-ocr` is the language-pack-less core; for non-English
        # OCR users will need `tesseract-ocr-traineddata-<lang>` separately.
        PKG_MGR="zypper"
        PKG_UPDATE_CMD="zypper refresh"
        PKG_INSTALL_CMD="zypper install -y"
        PKG_BUILD="gcc pkg-config libopenssl-devel git curl clang-devel perl make cmake alsa-devel"
        PKG_PDF="poppler-tools"
        PKG_OCR="tesseract-ocr"
        PKG_VIDEO="ffmpeg yt-dlp"
        PKG_DOCKER="docker"
        PKG_OPENSHELL_PREREQ="curl"
        PKG_TRADING_TOOLS="nodejs npm python3 python3-virtualenv"
        PKG_JAVA="maven unzip"
        JDK_CANDIDATES="java-25-openjdk-devel java-21-openjdk-devel"
        ;;
    alpine)
        # Alpine — common in containers. Note busybox `sh` already; the
        # script's POSIX-only constructs are fine. `--no-cache` skips
        # local index caching which is the standard apk convention.
        PKG_MGR="apk"
        PKG_UPDATE_CMD=""   # apk add --no-cache pulls fresh index per call
        PKG_INSTALL_CMD="apk add --no-cache"
        PKG_BUILD="build-base openssl-dev pkgconfig git curl clang perl make cmake alsa-lib-dev"
        PKG_PDF="poppler-utils"
        PKG_OCR="tesseract-ocr"
        PKG_VIDEO="ffmpeg yt-dlp"
        PKG_DOCKER="docker"
        PKG_OPENSHELL_PREREQ="curl"
        PKG_TRADING_TOOLS="nodejs npm python3 py3-virtualenv"
        PKG_JAVA="maven unzip"
        JDK_CANDIDATES="openjdk25 openjdk21"
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
        # Homebrew's gradle tracks upstream; the JDK comes from the temurin
        # cask rather than the keg-only `openjdk` formula, which needs a
        # root-owned symlink into /Library/Java before any build tool sees it.
        PKG_JAVA="maven gradle"
        GRADLE_FROM_PKG_MGR=true
        # Version-pinned rather than the bare `temurin` cask, which tracks the
        # newest feature release — see MACOS_JDK_CASK below.
        MACOS_JDK_CASK="temurin@25"
        ;;
    *)
        echo "install-system-deps.sh: unsupported OS (uname=$UNAME_S, distro=$DISTRO_ID)" >&2
        echo "  Supported: ubuntu/debian/wsl2, fedora/rhel/rocky/centos/almalinux, amazon-linux-2023, arch/manjaro, opensuse/sles, alpine, macos" >&2
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
    CHECK_BINS="gcc cc pkg-config git pdftotext pdfimages pdftoppm ffmpeg ffprobe yt-dlp"
    if [ "$TESSERACT_UNPACKAGED" = false ]; then
        CHECK_BINS="$CHECK_BINS tesseract"
    elif ! command -v tesseract >/dev/null 2>&1; then
        echo "install-system-deps.sh: note — tesseract is not packaged on $DISTRO_ID; image OCR needs the RapidOCR fallback (re-run with --with-ocr) or a source build" >&2
    fi
    for bin in $CHECK_BINS; do
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
    if [ "$WITH_JAVA" = true ]; then
        # javac as well as java: a JRE satisfies `java` and cannot compile,
        # which is the failure this check exists to catch early.
        for bin in java javac mvn gradle; do
            if ! command -v "$bin" >/dev/null 2>&1; then
                MISSING="$MISSING $bin"
            fi
        done
    fi
    if [ "$WITH_RUST" = true ] && ! have_cargo; then
        MISSING="$MISSING cargo"
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
        elif [ "$WITH_JAVA" = true ]; then
            [ "$PKG_MGR" = "brew" ] && echo "  Run: $0 --with-java" >&2 || echo "  Run: sudo $0 --with-java" >&2
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
    if [ "$WITH_JAVA" = true ]; then
        PRESENT="$PRESENT, java, javac, mvn, gradle"
    fi
    if [ "$WITH_RUST" = true ]; then
        PRESENT="$PRESENT, cargo"
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
# The newest OpenJDK package the distribution actually carries.
#
# A single pinned version is wrong on any release that does not carry it, and
# the distribution's own default is frequently several LTS versions behind
# (Ubuntu 24.04 still defaults to 21). So the candidates are tried newest-first
# and the first that exists wins. The last candidate in each list is one that is
# always present, so this cannot come back empty.
#
# `JDK_CANDIDATES` is empty where the package manager's JDK package already
# tracks the newest release (Arch) or the JDK does not come from the package
# manager at all (macOS, where it is a cask).
resolve_jdk_package() {
    if [ -z "$JDK_CANDIDATES" ]; then
        return 0
    fi
    for candidate in $JDK_CANDIDATES; do
        case "$PKG_MGR" in
            apt)    apt-cache show "$candidate" >/dev/null 2>&1 || continue ;;
            dnf)    dnf info "$candidate" >/dev/null 2>&1 || continue ;;
            zypper) zypper --non-interactive info "$candidate" 2>/dev/null | grep -q '^Version' || continue ;;
            apk)    apk info "$candidate" >/dev/null 2>&1 || continue ;;
            *)      : ;;
        esac
        echo "$candidate"
        return 0
    done
    # Every candidate probe failed — most likely because the package index has
    # never been fetched. Fall back to the last candidate rather than silently
    # installing no JDK at all, and let the package manager report the real
    # error.
    for candidate in $JDK_CANDIDATES; do
        LAST_JDK_CANDIDATE="$candidate"
    done
    echo "$LAST_JDK_CANDIDATE"
}

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
if [ "$WITH_JAVA" = true ]; then
    ALL_PKGS="$ALL_PKGS $PKG_JAVA $(resolve_jdk_package)"
fi
# Trim leading space if PKG_BUILD was empty (macOS case)
ALL_PKGS=$(echo "$ALL_PKGS" | sed 's/^ *//')

# dnf-family systems (Fedora/RHEL 9+, Amazon Linux 2023) preinstall
# curl-minimal, which CONFLICTS with the full `curl` package — requesting
# `curl` fails the whole transaction with "conflicts with curl provided by
# curl-minimal". curl-minimal already ships a fully HTTPS-capable curl
# binary, so never request `curl` there; if no curl binary exists at all
# (minimal containers), request the conflict-free curl-minimal flavor.
if [ "$PKG_MGR" = "dnf" ] && ! command -v curl >/dev/null 2>&1; then
    ALL_PKGS="$ALL_PKGS curl-minimal"
fi

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
echo "install-system-deps.sh: java toolchain: $WITH_JAVA"
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

# ---------------------------------------------------------------------------
# RapidOCR helper virtualenv
# ---------------------------------------------------------------------------
#
# `~/.archon-marker-venv` is the path archon itself probes for a RapidOCR
# interpreter (crates/archon-docs/src/ocr/rapid.rs), so creating it here is what
# makes image OCR work with no environment variable and no config.
#
# It has to be a virtualenv rather than a `pip install --user`. Debian 12+,
# Ubuntu 23.04+, Fedora 38+, Arch and Homebrew all ship an
# externally-managed-environment marker (PEP 668), and pip refuses to touch the
# system interpreter on those — the advice this script used to print,
# `python3 -m pip install rapidocr opencv-python`, fails outright on every one
# of them.
MARKER_VENV="$RUST_HOME/.archon-marker-venv"

install_marker_venv() {
    if [ "$WITH_OCR" != true ]; then
        return 0
    fi
    if ! command -v python3 >/dev/null 2>&1; then
        echo "install-system-deps.sh: python3 not found; skipping the RapidOCR venv" >&2
        return 0
    fi

    VENV_PY="$MARKER_VENV/bin/python"
    if [ -x "$VENV_PY" ]; then
        echo "install-system-deps.sh: RapidOCR venv already present at $MARKER_VENV"
    else
        # `python3 -m venv` is stdlib everywhere except Debian/Ubuntu, which
        # split it into python3-venv; that package is already in
        # PKG_TRADING_TOOLS there and is installed below when needed.
        if ! run python3 -m venv "$MARKER_VENV"; then
            echo "install-system-deps.sh: could not create $MARKER_VENV" >&2
            echo "  On Debian/Ubuntu install the venv module first: sudo apt-get install -y python3-venv" >&2
            return 0
        fi
    fi

    run "$VENV_PY" -m pip install --upgrade pip
    # opencv-python-headless rather than opencv-python: this venv never opens a
    # window, and the GUI build pulls a long tail of X11/GTK shared libraries
    # that are absent on servers and in containers.
    run "$VENV_PY" -m pip install rapidocr opencv-python-headless

    if [ -n "$RUST_USER" ] && [ "$DRY_RUN" = false ]; then
        # Created under sudo, but owned by the person who will run archon.
        run chown -R "$RUST_USER" "$MARKER_VENV"
    fi
}

install_rustup() {
    if [ "$WITH_RUST" != true ]; then
        return 0
    fi
    if have_cargo; then
        echo "install-system-deps.sh: cargo already present (rustup install skipped)"
        return 0
    fi
    RUSTUP_CMD="curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y"
    if [ "$DRY_RUN" = true ]; then
        if [ -n "$RUST_USER" ]; then
            echo "[dry-run] sudo -u $RUST_USER -H sh -c \"$RUSTUP_CMD\""
        else
            echo "[dry-run] $RUSTUP_CMD"
        fi
        return 0
    fi
    if [ -n "$RUST_USER" ]; then
        # Running under sudo: install as the invoking user so ~/.cargo lands
        # in their home, not root's.
        echo "+ installing rustup as user $RUST_USER"
        sudo -u "$RUST_USER" -H sh -c "$RUSTUP_CMD" || {
            echo "install-system-deps.sh: rustup install failed" >&2
            exit 3
        }
    else
        echo "+ installing rustup"
        sh -c "$RUSTUP_CMD" || {
            echo "install-system-deps.sh: rustup install failed" >&2
            exit 3
        }
    fi
}

# macOS JDK. Homebrew's `openjdk` formula is keg-only: it installs, and then no
# build tool finds it until a root-owned symlink is placed in
# /Library/Java/JavaVirtualMachines. The temurin cask installs a system JDK
# that Gradle and Maven detect with no further steps, which is why it is used
# here in preference to the formula.
install_java_macos() {
    if [ "$WITH_JAVA" != true ] || [ "$PKG_MGR" != "brew" ]; then
        return 0
    fi
    if command -v javac >/dev/null 2>&1; then
        echo "install-system-deps.sh: JDK already present ($(javac -version 2>&1))"
        return 0
    fi
    if [ "$DRY_RUN" = true ]; then
        echo "[dry-run] brew install --cask $MACOS_JDK_CASK"
        return 0
    fi
    echo "+ brew install --cask $MACOS_JDK_CASK"
    brew install --cask "$MACOS_JDK_CASK" || {
        echo "install-system-deps.sh: Temurin JDK install failed" >&2
        exit 3
    }
}

# Gradle, from the official distribution, on Linux.
#
# The distro packages are not usable for this: Ubuntu 22.04 ships Gradle 4.4.1
# and Fedora/RHEL/Amazon Linux do not package Gradle at all, so a project on any
# current Gradle would fail to configure. Arch and Homebrew track upstream and
# are handled through the package manager instead (GRADLE_FROM_PKG_MGR).
#
# The version is resolved from Gradle's own current-version endpoint rather than
# pinned here, so this does not go stale, and the download is checked against the
# published SHA-256 before anything is unpacked.
GRADLE_INSTALL_ROOT="/opt/gradle"

install_gradle() {
    if [ "$WITH_JAVA" != true ] || [ "$GRADLE_FROM_PKG_MGR" = true ] || [ "$PKG_MGR" = "brew" ]; then
        return 0
    fi
    if command -v gradle >/dev/null 2>&1; then
        echo "install-system-deps.sh: gradle already present ($(gradle --version 2>/dev/null | awk '/^Gradle /{print $2; exit}'))"
        return 0
    fi
    if [ "$DRY_RUN" = true ]; then
        echo "[dry-run] resolve https://services.gradle.org/versions/current, verify SHA-256, unpack to $GRADLE_INSTALL_ROOT, link /usr/local/bin/gradle"
        return 0
    fi

    GRADLE_META=$(curl -fsSL https://services.gradle.org/versions/current) || {
        echo "install-system-deps.sh: could not reach services.gradle.org to resolve the current Gradle version" >&2
        exit 3
    }
    GRADLE_URL=$(echo "$GRADLE_META" | sed -n 's/.*"downloadUrl"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p')
    GRADLE_SHA=$(echo "$GRADLE_META" | sed -n 's/.*"checksum"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p')
    GRADLE_VER=$(echo "$GRADLE_META" | sed -n 's/.*"version"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p')
    if [ -z "$GRADLE_URL" ] || [ -z "$GRADLE_SHA" ] || [ -z "$GRADLE_VER" ]; then
        echo "install-system-deps.sh: could not parse the Gradle version metadata" >&2
        exit 3
    fi

    echo "+ installing Gradle $GRADLE_VER from $GRADLE_URL"
    GRADLE_TMP=$(mktemp -d)
    curl -fsSL "$GRADLE_URL" -o "$GRADLE_TMP/gradle.zip" || {
        echo "install-system-deps.sh: Gradle download failed" >&2
        rm -rf "$GRADLE_TMP"
        exit 3
    }

    # Verify before unpacking, not after: an unverified archive is never
    # extracted onto the filesystem.
    ACTUAL_SHA=$(sha256sum "$GRADLE_TMP/gradle.zip" 2>/dev/null | cut -d' ' -f1)
    [ -n "$ACTUAL_SHA" ] || ACTUAL_SHA=$(shasum -a 256 "$GRADLE_TMP/gradle.zip" 2>/dev/null | cut -d' ' -f1)
    if [ "$ACTUAL_SHA" != "$GRADLE_SHA" ]; then
        echo "install-system-deps.sh: Gradle checksum mismatch — refusing to install" >&2
        echo "  expected: $GRADLE_SHA" >&2
        echo "  actual:   ${ACTUAL_SHA:-<could not compute>}" >&2
        rm -rf "$GRADLE_TMP"
        exit 3
    fi

    $SUDO mkdir -p "$GRADLE_INSTALL_ROOT" || {
        echo "install-system-deps.sh: could not create $GRADLE_INSTALL_ROOT" >&2
        rm -rf "$GRADLE_TMP"
        exit 3
    }
    $SUDO unzip -q -o -d "$GRADLE_INSTALL_ROOT" "$GRADLE_TMP/gradle.zip" || {
        echo "install-system-deps.sh: Gradle archive extraction failed (is unzip installed?)" >&2
        rm -rf "$GRADLE_TMP"
        exit 3
    }
    rm -rf "$GRADLE_TMP"

    $SUDO ln -sfn "$GRADLE_INSTALL_ROOT/gradle-$GRADLE_VER/bin/gradle" /usr/local/bin/gradle || {
        echo "install-system-deps.sh: could not link gradle into /usr/local/bin" >&2
        exit 3
    }
}

install_amzn_extras() {
    if [ "$AMZN_BINARY_FALLBACKS" != true ]; then
        return 0
    fi

    # ffmpeg + ffprobe: static builds from johnvansickle.com (the standard
    # source for Amazon Linux, linked from ffmpeg.org). Arch-aware.
    if command -v ffmpeg >/dev/null 2>&1 && command -v ffprobe >/dev/null 2>&1; then
        echo "install-system-deps.sh: ffmpeg/ffprobe already present"
    else
        case "$HOST_ARCH" in
            x86_64)  FFMPEG_ARCH="amd64" ;;
            aarch64) FFMPEG_ARCH="arm64" ;;
            *)
                echo "install-system-deps.sh: no static ffmpeg build for arch $HOST_ARCH — install ffmpeg manually" >&2
                exit 3
                ;;
        esac
        FFMPEG_URL="https://johnvansickle.com/ffmpeg/releases/ffmpeg-release-${FFMPEG_ARCH}-static.tar.xz"
        if [ "$DRY_RUN" = true ]; then
            echo "[dry-run] curl -fsSL $FFMPEG_URL | tar -xJ, then install ffmpeg+ffprobe to /usr/local/bin"
        else
            echo "+ installing static ffmpeg from $FFMPEG_URL"
            FFMPEG_TMP=$(mktemp -d)
            curl -fsSL "$FFMPEG_URL" -o "$FFMPEG_TMP/ffmpeg.tar.xz" || {
                echo "install-system-deps.sh: ffmpeg static build download failed" >&2
                rm -rf "$FFMPEG_TMP"
                exit 3
            }
            tar -xJf "$FFMPEG_TMP/ffmpeg.tar.xz" -C "$FFMPEG_TMP" || {
                echo "install-system-deps.sh: ffmpeg archive extraction failed" >&2
                rm -rf "$FFMPEG_TMP"
                exit 3
            }
            FFMPEG_DIR=$(find "$FFMPEG_TMP" -maxdepth 1 -type d -name "ffmpeg-*-static" | head -1)
            $SUDO install -m 0755 "$FFMPEG_DIR/ffmpeg" "$FFMPEG_DIR/ffprobe" /usr/local/bin/ || {
                echo "install-system-deps.sh: ffmpeg install to /usr/local/bin failed" >&2
                rm -rf "$FFMPEG_TMP"
                exit 3
            }
            rm -rf "$FFMPEG_TMP"
        fi
    fi

    # yt-dlp: official standalone binary (self-updatable via -U). Use the
    # arch-specific PyInstaller builds, NOT the generic `yt-dlp` zipapp —
    # the zipapp runs on the system python3, and AL2023's default Python is
    # older than yt-dlp supports (it tracebacks on --version). The health
    # check below also replaces an existing-but-broken zipapp install.
    if command -v yt-dlp >/dev/null 2>&1 && yt-dlp --version >/dev/null 2>&1; then
        echo "install-system-deps.sh: yt-dlp already present"
    else
        if command -v yt-dlp >/dev/null 2>&1; then
            echo "install-system-deps.sh: existing yt-dlp is broken (--version fails); reinstalling standalone build"
        fi
        case "$HOST_ARCH" in
            x86_64)  YTDLP_ASSET="yt-dlp_linux" ;;
            aarch64) YTDLP_ASSET="yt-dlp_linux_aarch64" ;;
            *)       YTDLP_ASSET="yt-dlp" ;;   # zipapp fallback; needs modern python3
        esac
        YTDLP_URL="https://github.com/yt-dlp/yt-dlp/releases/latest/download/$YTDLP_ASSET"
        if [ "$DRY_RUN" = true ]; then
            echo "[dry-run] curl -fsSL $YTDLP_URL -o /usr/local/bin/yt-dlp && chmod +x"
        else
            echo "+ installing yt-dlp from $YTDLP_URL"
            $SUDO curl -fsSL "$YTDLP_URL" -o /usr/local/bin/yt-dlp || {
                echo "install-system-deps.sh: yt-dlp download failed" >&2
                exit 3
            }
            $SUDO chmod 0755 /usr/local/bin/yt-dlp
        fi
    fi

    echo "install-system-deps.sh: note — tesseract is not packaged on Amazon Linux 2023."
    echo "  For image OCR use the RapidOCR fallback: python3 -m pip install rapidocr opencv-python"
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
install_amzn_extras
install_java_macos
install_gradle
install_rustup
install_marker_venv
install_openshell
setup_openshell_gateway

# ---------------------------------------------------------------------------
# Post-install verification
# ---------------------------------------------------------------------------
if [ "$DRY_RUN" = false ]; then
    echo
    echo "install-system-deps.sh: verifying installs..."
    VERIFY_BINS="pdftotext pdfimages pdftoppm ffmpeg ffprobe yt-dlp"
    if [ "$TESSERACT_UNPACKAGED" = false ]; then
        VERIFY_BINS="pdftotext pdfimages pdftoppm tesseract ffmpeg ffprobe yt-dlp"
    fi
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
    if [ "$WITH_JAVA" = true ]; then
        VERIFY_BINS="$VERIFY_BINS java javac mvn gradle"
    fi
    for bin in $VERIFY_BINS; do
        if command -v "$bin" >/dev/null 2>&1; then
            # poppler utilities only understand -v (--version is read as a
            # filename and errors); everything else takes --version.
            case "$bin" in
                pdftotext|pdfimages|pdftoppm)
                    VERSION=$("$bin" -v 2>&1 | head -n 1 || echo "(version check failed)") ;;
                gradle)
                    # `gradle --version` prints a banner; the version is on a
                    # later line, and the whole thing needs a JVM to start.
                    VERSION=$("$bin" --version 2>&1 | awk '/^Gradle /{print; exit}' || echo "(version check failed)")
                    [ -n "$VERSION" ] || VERSION="(version check failed)" ;;
                java|javac)
                    # Both write their version banner to stderr, not stdout.
                    VERSION=$("$bin" -version 2>&1 | head -n 1 || echo "(version check failed)") ;;
                *)
                    VERSION=$("$bin" --version 2>&1 | head -n 1 || echo "(version check failed)") ;;
            esac
            echo "  ok: $bin     $VERSION"
        else
            echo "  MISSING: $bin (post-install check failed)" >&2
        fi
    done
    if [ "$WITH_OCR" = true ]; then
        if [ -x "$MARKER_VENV/bin/python" ] \
            && "$MARKER_VENV/bin/python" -c "import rapidocr" >/dev/null 2>&1; then
            echo "  ok: rapidocr ($MARKER_VENV)"
        else
            echo "  MISSING: rapidocr venv at $MARKER_VENV (post-install check failed)" >&2
        fi
    fi
    if [ "$WITH_RUST" = true ]; then
        if have_cargo; then
            echo "  ok: cargo    ($RUST_HOME/.cargo/bin/cargo)"
        else
            echo "  MISSING: cargo (rustup post-install check failed)" >&2
        fi
    fi
    echo
    echo "install-system-deps.sh: done. Next steps:"
    if [ "$WITH_RUST" = true ]; then
        echo "  1. Load cargo into this shell: . \"$RUST_HOME/.cargo/env\"  (new shells pick it up automatically)"
    else
        echo "  1. Install rustup (--no-rust was given; the repo pins its toolchain; rustup fetches it): curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh"
    fi
    echo "  2. Build archon-cli: cargo build --release --bin archon"
    echo "  3. Initialise a project: ./scripts/archon-init.sh --target /path/to/project"
    echo "  4. For local video ASR, download a whisper.cpp model and set [policy.video.asr].model"
    if [ "$WITH_OCR" = true ]; then
        echo "     RapidOCR image OCR is installed at $MARKER_VENV and needs no further configuration"
    else
        echo "     Optional RapidOCR image-OCR fallback: re-run this script with --with-ocr"
    fi
    echo "     Optional Trading Lab tools: scripts/setup-trading-tools.sh --target /path/to/project"
    if [ "$WITH_JAVA" = true ]; then
        echo "     Java analysis needs no further system packages: Checkstyle, PMD, SpotBugs,"
        echo "     FindSecBugs, Error Prone and PIT are declared by the project's own build."
    else
        echo "     Optional Java toolchain (JDK, Gradle, Maven): re-run this script with --with-java"
    fi
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
