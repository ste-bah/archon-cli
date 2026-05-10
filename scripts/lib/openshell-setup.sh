#!/bin/sh
# OpenShell install and gateway setup helpers for install-system-deps.sh.

OPENSHELL_INSTALL_URL="https://raw.githubusercontent.com/NVIDIA/OpenShell/main/install.sh"
OPENSHELL_LOCAL_GATEWAY_URL="${OPENSHELL_LOCAL_GATEWAY_URL:-https://127.0.0.1:17670}"
OPENSHELL_HOMEBREW_FORMULA="${OPENSHELL_HOMEBREW_FORMULA:-nvidia/openshell/openshell}"

openshell_add_default_path() {
    for dir in "${HOME:-}/.local/bin" /opt/homebrew/bin /usr/local/bin; do
        [ -d "$dir" ] || continue
        case ":$PATH:" in
            *":$dir:"*) ;;
            *) PATH="$dir:$PATH"; export PATH ;;
        esac
    done
}

run_openshell_installer() {
    if [ "$DRY_RUN" = true ]; then
        if command -v curl >/dev/null 2>&1; then
            echo "[dry-run] curl -LsSf $OPENSHELL_INSTALL_URL | sh"
        else
            echo "[dry-run] wget -qO- $OPENSHELL_INSTALL_URL | sh"
        fi
        return 0
    fi
    if command -v curl >/dev/null 2>&1; then
        echo "+ curl -LsSf $OPENSHELL_INSTALL_URL | sh"
        curl -LsSf "$OPENSHELL_INSTALL_URL" | sh || {
            echo "install-system-deps.sh: OpenShell install script failed" >&2
            exit 3
        }
    elif command -v wget >/dev/null 2>&1; then
        echo "+ wget -qO- $OPENSHELL_INSTALL_URL | sh"
        wget -qO- "$OPENSHELL_INSTALL_URL" | sh || {
            echo "install-system-deps.sh: OpenShell install script failed" >&2
            exit 3
        }
    else
        echo "install-system-deps.sh: cannot install OpenShell because curl/wget is unavailable" >&2
        exit 3
    fi
}

install_openshell() {
    [ "$WITH_OPENSHELL" = true ] || return 0
    openshell_add_default_path
    if command -v openshell >/dev/null 2>&1 && [ "$SETUP_OPENSHELL_GATEWAY" != true ]; then
        echo "install-system-deps.sh: openshell already present"
        return 0
    fi
    if command -v openshell >/dev/null 2>&1; then
        echo "install-system-deps.sh: refreshing OpenShell with official NVIDIA installer"
    else
        echo "install-system-deps.sh: installing OpenShell with official NVIDIA installer"
    fi
    run_openshell_installer
    [ "$DRY_RUN" = true ] && return 0
    openshell_add_default_path
    command -v openshell >/dev/null 2>&1 || {
        echo "install-system-deps.sh: openshell is still not on PATH after installation" >&2
        exit 3
    }
}

restart_openshell_service() {
    case "$OS_FAMILY" in
        macos)
            command -v brew >/dev/null 2>&1 || return 1
            echo "+ brew services restart $OPENSHELL_HOMEBREW_FORMULA"
            brew services restart "$OPENSHELL_HOMEBREW_FORMULA" \
                || brew services restart openshell \
                || return 1
            ;;
        linux)
            command -v systemctl >/dev/null 2>&1 || return 1
            echo "+ systemctl --user enable openshell-gateway"
            systemctl --user daemon-reload || return 1
            systemctl --user enable openshell-gateway || return 1
            echo "+ systemctl --user restart openshell-gateway"
            systemctl --user restart openshell-gateway || return 1
            ;;
        *)
            return 1
            ;;
    esac
}

register_openshell_gateway() {
    openshell gateway add "$OPENSHELL_LOCAL_GATEWAY_URL" --local --name openshell >/dev/null 2>&1 \
        || true
}

openshell_status_ready() {
    timeout="${OPENSHELL_INSTALL_GATEWAY_TIMEOUT:-30}"
    elapsed=0
    while [ "$elapsed" -lt "$timeout" ]; do
        openshell status >/dev/null 2>&1 && return 0
        sleep 1
        elapsed=$((elapsed + 1))
    done
    return 1
}

setup_openshell_gateway() {
    [ "$SETUP_OPENSHELL_GATEWAY" = true ] || return 0
    if [ "$DRY_RUN" = true ]; then
        echo "[dry-run] docker info"
        echo "[dry-run] openshell status || restart/register local OpenShell gateway service"
        echo "[dry-run] openshell status || openshell gateway start (legacy CLI only)"
        return 0
    fi
    if [ "$(id -u 2>/dev/null || echo 1)" -eq 0 ]; then
        echo "install-system-deps.sh: refusing OpenShell gateway setup as root" >&2
        echo "  Re-run as your normal user: $0 --with-openshell --setup-openshell-gateway" >&2
        exit 1
    fi
    for bin in docker openshell; do
        command -v "$bin" >/dev/null 2>&1 || {
            echo "install-system-deps.sh: $bin is required before OpenShell gateway setup" >&2
            exit 3
        }
    done
    if ! docker info >/dev/null 2>&1; then
        echo "install-system-deps.sh: Docker is installed but the daemon is not reachable" >&2
        echo "  Start Docker Desktop or Docker Engine, then re-run this command." >&2
        exit 3
    fi
    if openshell status >/dev/null 2>&1; then
        echo "install-system-deps.sh: OpenShell gateway already active"
        openshell status || true
        return 0
    fi
    restart_openshell_service && register_openshell_gateway || true
    if openshell_status_ready; then
        openshell status || true
        return 0
    fi
    if openshell gateway start --help >/dev/null 2>&1; then
        echo "+ openshell gateway start"
        openshell gateway start || {
            echo "install-system-deps.sh: OpenShell gateway start failed" >&2
            exit 3
        }
        openshell_status_ready || {
            echo "install-system-deps.sh: OpenShell gateway status check failed" >&2
            exit 3
        }
        openshell status || true
        return 0
    fi
    echo "install-system-deps.sh: OpenShell gateway is not active" >&2
    echo "  macOS: brew services restart $OPENSHELL_HOMEBREW_FORMULA" >&2
    echo "  Linux: systemctl --user enable openshell-gateway && systemctl --user restart openshell-gateway" >&2
    echo "  Register: openshell gateway add $OPENSHELL_LOCAL_GATEWAY_URL --local --name openshell" >&2
    echo "  Then verify with: openshell status" >&2
    exit 3
}
