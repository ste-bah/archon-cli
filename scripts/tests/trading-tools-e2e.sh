#!/bin/sh
set -eu
ROOT=$(CDPATH= cd "$(dirname "$0")/../.." && pwd)
TARGET="${RUNNER_TEMP:-${TMPDIR:-/tmp}}/trading tools with spaces"
mkdir -p "$TARGET"
install_output=$("$ROOT/scripts/setup-trading-tools.sh" --target "$TARGET" 2>&1) || {
    printf '%s\n' "$install_output" >&2
    echo 'Installer assertion failed: TradingView-enabled setup did not complete' >&2
    exit 1
}
printf '%s\n' "$install_output"
if [ ! -f "$TARGET/.archon/tools/tradingview-mcp/src/server.js" ]; then
    echo 'TradingView assertion failed: installer completed without the server entry point' >&2; exit 1;
fi
printf '%s\n' "$install_output" | grep -F 'OpenBB venv: healthy' >/dev/null || {
    echo 'Installer assertion failed: final OpenBB status was not healthy' >&2; exit 1;
}
VENV="$TARGET/.archon/tools/openbb-venv"
if openbb_import_output=$("$VENV/bin/python" -c 'import openbb' 2>&1); then
    openbb_import_exit=0
else
    openbb_import_exit=$?
    printf 'OpenBB import failed with exit %s; last output lines:\n%s\n' \
        "$openbb_import_exit" "$(printf '%s\n' "$openbb_import_output" | tail -n 20)" >&2
    echo 'Import assertion failed: venv Python could not import openbb' >&2
    exit 1
fi
ARCHON_OPENBB_IMPORT_EXIT="$openbb_import_exit" ARCHON_OPENBB_IMPORT_OUTPUT="$openbb_import_output" python3 - "$TARGET" <<'PY'
import os, pathlib, signal, socket, subprocess, sys, time, urllib.error, urllib.request
target = pathlib.Path(sys.argv[1])
venv_path = str((target / ".archon/tools/openbb-venv").resolve())
launcher = target / "scripts/start-openbb-api.sh"
log_path = target / "openbb-api.log"
with socket.socket() as listener:
    listener.bind(("127.0.0.1", 0))
    port = listener.getsockname()[1]
env = os.environ.copy()
env.update(OPENBB_HOST="127.0.0.1", OPENBB_PORT=str(port))
with log_path.open("wb") as log:
    process = subprocess.Popen([str(launcher)], cwd=target, env=env, stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
try:
    deadline = time.monotonic() + 90
    while time.monotonic() < deadline:
        if process.poll() is not None:
            raise RuntimeError("OpenBB API server exited before /docs returned HTTP 200")
        try:
            with urllib.request.urlopen(f"http://127.0.0.1:{port}/docs", timeout=2) as response:
                if response.status == 200:
                    print(f"PASS OpenBB API /docs returned HTTP 200 on port {port}")
                    break
        except (OSError, urllib.error.URLError):
            time.sleep(1)
    else:
        raise RuntimeError("OpenBB API /docs did not return HTTP 200 within 90 seconds")
except Exception as error:
    print(f"API assertion failed: {error}", file=sys.stderr)
    print("OpenBB import probe exit:", os.environ.get("ARCHON_OPENBB_IMPORT_EXIT", "unknown"), file=sys.stderr)
    print("\n".join(os.environ.get("ARCHON_OPENBB_IMPORT_OUTPUT", "").splitlines()[-20:]), file=sys.stderr)
    print("OpenBB API server log:", file=sys.stderr)
    print(log_path.read_text(errors="replace"), file=sys.stderr)
    raise SystemExit(1)
finally:
    pgid = process.pid
    try:
        os.killpg(pgid, signal.SIGTERM)
        print(f"API teardown: sent SIGTERM to process group {pgid}")
    except ProcessLookupError:
        print(f"API teardown: process group {pgid} had already stopped")
    try:
        process.wait(timeout=5)
    except subprocess.TimeoutExpired:
        pass
    deadline = time.monotonic() + 30
    while time.monotonic() < deadline:
        # The separate session gives the API its own group. Kill anything that
        # remains in it, and also check process command paths for venv users.
        try:
            os.killpg(pgid, 0)
        except ProcessLookupError:
            group_alive = False
        else:
            group_alive = True
            try:
                os.killpg(pgid, signal.SIGKILL)
                print(f"API teardown: sent SIGKILL to remaining process group {pgid}")
            except ProcessLookupError:
                group_alive = False
        ps = subprocess.run(["ps", "-axo", "pid=,command="], check=False, text=True, capture_output=True)
        venv_pids = []
        for line in ps.stdout.splitlines():
            fields = line.strip().split(None, 1)
            if len(fields) == 2 and fields[0].isdigit() and int(fields[0]) != os.getpid() and venv_path in fields[1]:
                venv_pids.append((int(fields[0]), fields[1]))
        for pid, command in venv_pids:
            print(f"API teardown: stopping remaining venv process PID {pid}: {command}")
            try:
                os.kill(pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
        if not group_alive and not venv_pids:
            print("API teardown: no processes remain from the OpenBB venv")
            break
        time.sleep(0.5)
    else:
        raise RuntimeError("Timed out waiting for OpenBB API process group and venv processes to stop")
PY
rm -f "$VENV/bin/python"
broken_status=$("$ROOT/scripts/setup-trading-tools.sh" --target "$TARGET" --check 2>&1) || {
    printf '%s\n' "$broken_status" >&2
    echo 'Repair assertion failed: status check failed for missing venv interpreter' >&2
    exit 1
}
printf '%s\n' "$broken_status" | grep -F 'OpenBB venv: broken' >/dev/null || {
    printf '%s\n' "$broken_status" >&2
    echo 'Repair assertion failed: installer did not report the missing interpreter as broken' >&2
    exit 1
}
echo 'PASS installer reported broken OpenBB venv after interpreter deletion'
broken_output=$("$ROOT/scripts/setup-trading-tools.sh" --target "$TARGET" 2>&1) || {
    printf '%s\n' "$broken_output" >&2
    echo 'Repair assertion failed: installer failed to rebuild a broken OpenBB venv' >&2
    exit 1
}
printf '%s\n' "$broken_output"
printf '%s\n' "$broken_output" | grep -F 'OpenBB venv: healthy' >/dev/null || {
    echo 'Repair assertion failed: rebuilt OpenBB status was not healthy' >&2; exit 1;
}
if openbb_import_output=$("$VENV/bin/python" -c 'import openbb' 2>&1); then
    :
else
    openbb_import_exit=$?
    printf 'Repair OpenBB import failed with exit %s; last output lines:\n%s\n' \
        "$openbb_import_exit" "$(printf '%s\n' "$openbb_import_output" | tail -n 20)" >&2
    echo 'Repair assertion failed: rebuilt interpreter could not import openbb' >&2
    exit 1
fi
echo 'PASS broken OpenBB interpreter rebuilt and reported healthy'
echo 'test result: trading tools POSIX end-to-end passed'
