#!/bin/sh
set -eu
SCRIPT_DIR=$(CDPATH= cd "$(dirname "$0")" && pwd)
REPO_ROOT=$(CDPATH= cd "$SCRIPT_DIR/../.." && pwd)
TEMP_ROOT=$(mktemp -d "${TMPDIR:-/tmp}/trading setup.XXXXXX")
TEST_CASE='harness setup'
cleanup() {
    status=$?
    trap - EXIT HUP INT TERM
    rm -rf "$TEMP_ROOT"
    if [ "$status" -ne 0 ]; then
        printf 'FAIL: %s (exit %s)\n' "$TEST_CASE" "$status" >&2
    fi
    exit "$status"
}
trap cleanup EXIT
trap 'exit 129' HUP
trap 'exit 130' INT
trap 'exit 143' TERM
mkdir -p "$TEMP_ROOT/home"
HOME="$TEMP_ROOT/home"
export HOME
TARGET="$TEMP_ROOT/project with spaces"
mkdir -p "$TARGET"
TARGET=$(cd "$TARGET" && pwd)

for mode in --check --dry-run; do
    TEST_CASE="status $mode with spaced target"
    output=$($REPO_ROOT/scripts/setup-trading-tools.sh --target "$TARGET" "$mode" --skip-tradingview --skip-openbb)
    printf '%s\n' "$output" | grep -F "target=$TARGET" >/dev/null
    printf '%s\n' "$output" | grep -F 'OpenBB venv: missing' >/dev/null
    test ! -e "$TARGET/.archon/tools"
    test ! -e "$TARGET/scripts"
done

# The installer must source a user's profile before checking the tool PATH.
TEST_CASE='profile PATH loading'
PROFILE_BIN="$TEMP_ROOT/profile bin"
mkdir -p "$PROFILE_BIN"
cat > "$PROFILE_BIN/node" <<'EOF'
#!/bin/sh
echo 'profile-node'
EOF
chmod +x "$PROFILE_BIN/node"
printf 'PATH="%s:$PATH"\n' "$PROFILE_BIN" > "$HOME/.profile"
profile_output=$("$REPO_ROOT/scripts/setup-trading-tools.sh" --target "$TARGET" --check --skip-tradingview --skip-openbb)
printf '%s\n' "$profile_output" | grep -F 'profile-node' >/dev/null
rm -f "$HOME/.profile"

# Nounset in a profile must not abort setup, and a later profile command can
# still add tools to PATH even when sourcing eventually returns non-zero.
TEST_CASE='profile nounset and non-zero handling'
unset UNSET_VAR_FOR_TEST
cat > "$HOME/.profile" <<EOF
echo "\$UNSET_VAR_FOR_TEST" >/dev/null
PATH="$PROFILE_BIN:\$PATH"
false
EOF
if profile_output=$(PATH="$PATH" \
    "$REPO_ROOT/scripts/setup-trading-tools.sh" --target "$TARGET" --dry-run --skip-tradingview --skip-openbb 2>&1); then
    profile_status=0
else
    profile_status=$?
fi
test "$profile_status" -eq 0
printf '%s\n' "$profile_output" | grep -F 'profile-node' >/dev/null
printf '%s\n' "$profile_output" | grep -F "warning: could not fully source profile: $HOME/.profile" >/dev/null
rm -f "$HOME/.profile"

if "$REPO_ROOT/scripts/setup-trading-tools.sh" --target 2>/dev/null; then
    TEST_CASE='missing --target argument refusal'
    echo 'expected missing --target argument to fail' >&2
    exit 1
fi

FAKE_BIN="$TEMP_ROOT/fake bin"
mkdir -p "$FAKE_BIN"
cat > "$FAKE_BIN/python3" <<'EOF'
#!/bin/sh
if [ -n "${FAKE_SETUP_LOG:-}" ]; then
    printf '%s %s\n' "$(basename "$0")" "$*" >> "$FAKE_SETUP_LOG"
fi
if [ "$1" = --version ]; then echo 'Python 3.12.1'; exit 0; fi
if [ -f "$1" ] && [ -n "$ARCHON_SETUP_PYTHON_SPECIFIER" ]; then exit 0; fi
if [ "$1" = -c ]; then exit 0; fi
if [ "$1" = -m ] && [ "$2" = venv ]; then
    [ "$3" = --clear ] || exit 9
    if [ "${FAKE_VENV_CREATE:-}" = fail ]; then
        printf '%s\n' "$*" >> "$FAKE_SETUP_LOG"
        exit 0
    fi
    VENV="$4"
    mkdir -p "$VENV/bin"
    printf 'home = fake-python\n' > "$VENV/pyvenv.cfg"
    if [ -L "$VENV/bin/python" ]; then unlink "$VENV/bin/python"; fi
    cat > "$VENV/bin/python" <<'PYTHON'
#!/bin/sh
exit 0
PYTHON
    cat > "$VENV/bin/openbb-api" <<'API'
#!/bin/sh
printf '%s\n' "$*"
API
    chmod +x "$VENV/bin/python" "$VENV/bin/openbb-api"
    printf '%s\n' "$*" >> "$FAKE_SETUP_LOG"
    exit 0
fi
exit 8
EOF
chmod +x "$FAKE_BIN/python3"
for versioned_python in 3.20 3.19 3.18 3.17 3.16 3.15 3.14 3.13 3.12 3.11 3.10; do
    cp "$FAKE_BIN/python3" "$FAKE_BIN/python$versioned_python"
    chmod +x "$FAKE_BIN/python$versioned_python"
done
BROKEN_TARGET="$TEMP_ROOT/broken target"
TEST_CASE='broken OpenBB venv repair'
BROKEN_VENV="$BROKEN_TARGET/.archon/tools/openbb-venv"
mkdir -p "$BROKEN_VENV/bin"
printf 'home = fake-python\n' > "$BROKEN_VENV/pyvenv.cfg"
ln -s "$BROKEN_VENV/bin/removed-python" "$BROKEN_VENV/bin/python"
FAKE_SETUP_LOG="$TEMP_ROOT/venv invocation.log"
PATH="$FAKE_BIN:$PATH" FAKE_SETUP_LOG="$FAKE_SETUP_LOG" \
    "$REPO_ROOT/scripts/setup-trading-tools.sh" --target "$BROKEN_TARGET" --skip-tradingview
grep -F -- '-m venv --clear ' "$FAKE_SETUP_LOG" >/dev/null

# A successful base-interpreter exit without a created venv must name the
# base interpreter, venv path, and missing interpreter file clearly.
STORE_TARGET="$TEMP_ROOT/store stub target"
TEST_CASE='missing venv interpreter diagnostic'
STORE_VENV="$STORE_TARGET/.archon/tools/openbb-venv"
mkdir -p "$STORE_TARGET"
STORE_TARGET=$(cd "$STORE_TARGET" && pwd)
STORE_VENV="$STORE_TARGET/.archon/tools/openbb-venv"
expected_store_error="setup-trading-tools.sh: venv creation with base interpreter 'python3' did not create expected interpreter '$STORE_VENV/bin/python' in venv '$STORE_VENV'."
if store_output=$(PATH="$FAKE_BIN:$PATH" FAKE_SETUP_LOG="$FAKE_SETUP_LOG" FAKE_VENV_CREATE=fail \
    "$REPO_ROOT/scripts/setup-trading-tools.sh" --target "$STORE_TARGET" --skip-tradingview 2>&1); then
    echo 'expected missing venv interpreter to fail' >&2
    exit 1
fi
if ! printf '%s\n' "$store_output" | grep -Fx "$expected_store_error" >/dev/null; then
    printf 'expected exact missing-interpreter diagnostic:\n%s\nactual output:\n%s\n' \
        "$expected_store_error" "$store_output" >&2
    exit 1
fi
grep -F -- "python3 -m venv --clear $STORE_VENV" "$FAKE_SETUP_LOG" >/dev/null
test ! -e "$STORE_VENV/bin/python"

test -f "$BROKEN_TARGET/scripts/start-openbb-api.sh"
test -f "$BROKEN_TARGET/scripts/start-openbb-api.ps1"
grep -F "OPENBB_HOST" "$BROKEN_TARGET/scripts/start-openbb-api.ps1" >/dev/null
grep -F "OPENBB_PORT" "$BROKEN_TARGET/scripts/start-openbb-api.ps1" >/dev/null
grep -F ".archon/tools/openbb-venv/Scripts/openbb-api.exe" "$BROKEN_TARGET/scripts/start-openbb-api.ps1" >/dev/null
start_output=$(OPENBB_HOST=0.0.0.0 OPENBB_PORT=7011 "$BROKEN_TARGET/scripts/start-openbb-api.sh")
test "$start_output" = '--host 0.0.0.0 --port 7011'

LINK_TARGET="$TEMP_ROOT/linked venv target"
TEST_CASE='symlinked OpenBB venv refusal'
LINKED_VENV_TARGET="$TEMP_ROOT/linked project"
mkdir -p "$LINK_TARGET"
printf 'keep me\n' > "$LINK_TARGET/sentinel.txt"
mkdir -p "$LINKED_VENV_TARGET/.archon/tools"
ln -s "$LINK_TARGET" "$LINKED_VENV_TARGET/.archon/tools/openbb-venv"
: > "$FAKE_SETUP_LOG"
if link_output=$(PATH="$FAKE_BIN:$PATH" FAKE_SETUP_LOG="$FAKE_SETUP_LOG" \
    "$REPO_ROOT/scripts/setup-trading-tools.sh" --target "$LINKED_VENV_TARGET" --skip-tradingview 2>&1); then
    echo 'expected symlinked OpenBB venv to be refused' >&2
    exit 1
fi
printf '%s\n' "$link_output" | grep -F "$LINK_TARGET" >/dev/null
if [ -s "$FAKE_SETUP_LOG" ]; then
    echo "expected no interpreter calls for symlinked OpenBB venv, got: $(cat "$FAKE_SETUP_LOG")" >&2
    exit 1
fi
test "$(cat "$LINK_TARGET/sentinel.txt")" = 'keep me'

NON_VENV_TARGET="$TEMP_ROOT/non-venv project"
TEST_CASE='non-venv directory refusal'
NON_VENV="$NON_VENV_TARGET/.archon/tools/openbb-venv"
mkdir -p "$NON_VENV"
NON_VENV=$(cd "$NON_VENV" && pwd)
printf 'keep non-venv data\n' > "$NON_VENV/sentinel.txt"
if non_venv_output=$(PATH="$FAKE_BIN:$PATH" FAKE_SETUP_LOG="$FAKE_SETUP_LOG" \
    "$REPO_ROOT/scripts/setup-trading-tools.sh" --target "$NON_VENV_TARGET" --skip-tradingview 2>&1); then
    echo 'expected non-venv directory with files to be refused' >&2
    exit 1
fi
printf '%s\n' "$non_venv_output" | grep -F "$NON_VENV" >/dev/null
test "$(cat "$NON_VENV/sentinel.txt")" = 'keep non-venv data'

HEALTHY_TARGET="$TEMP_ROOT/healthy project"
TEST_CASE='healthy OpenBB venv preservation'
HEALTHY_VENV="$HEALTHY_TARGET/.archon/tools/openbb-venv"
mkdir -p "$HEALTHY_VENV/bin"
printf 'home = fake-python\n' > "$HEALTHY_VENV/pyvenv.cfg"
cat > "$HEALTHY_VENV/bin/python" <<'PYTHON'
#!/bin/sh
exit 0
PYTHON
chmod +x "$HEALTHY_VENV/bin/python"
healthy_cfg_before=$(cksum "$HEALTHY_VENV/pyvenv.cfg")
healthy_mtime_before=$(perl -e 'print((stat($ARGV[0]))[9])' "$HEALTHY_VENV/pyvenv.cfg")
: > "$FAKE_SETUP_LOG"
PATH="$FAKE_BIN:$PATH" FAKE_SETUP_LOG="$FAKE_SETUP_LOG" \
    "$REPO_ROOT/scripts/setup-trading-tools.sh" --target "$HEALTHY_TARGET" --skip-tradingview >/dev/null
healthy_cfg_after=$(cksum "$HEALTHY_VENV/pyvenv.cfg")
healthy_mtime_after=$(perl -e 'print((stat($ARGV[0]))[9])' "$HEALTHY_VENV/pyvenv.cfg")
test "$healthy_cfg_before" = "$healthy_cfg_after"
test "$healthy_mtime_before" = "$healthy_mtime_after"
if grep -F -- '-m venv --clear ' "$FAKE_SETUP_LOG" >/dev/null; then
    echo 'healthy OpenBB venv was unnecessarily rebuilt' >&2
    exit 1
fi

# A healthy venv must not need any base Python on PATH.
CORE_BIN="$TEMP_ROOT/core tools"
TEST_CASE='healthy venv without base Python on PATH'
mkdir -p "$CORE_BIN"
for core_tool in mkdir cat chmod; do
    ln -s "$(command -v "$core_tool")" "$CORE_BIN/$core_tool"
done
PATH="$CORE_BIN" FAKE_SETUP_LOG="$FAKE_SETUP_LOG" \
    "$REPO_ROOT/scripts/setup-trading-tools.sh" --target "$HEALTHY_TARGET" --skip-tradingview >/dev/null

# Exercise the full specifier against mocked candidate versions. A non-venv
# sentinel makes accepted candidates stop before any install or network call.
REAL_PYTHON=$(command -v python3)
SPEC_BIN="$TEMP_ROOT/spec interpreters"
MOCK_SYSTEM_BIN="$TEMP_ROOT/mock system tools"
mkdir -p "$SPEC_BIN"
mkdir -p "$MOCK_SYSTEM_BIN"
for system_tool in dirname mktemp mv rm cat find awk head grep sed mkdir chmod readlink tr; do
    ln -s "$(command -v "$system_tool")" "$MOCK_SYSTEM_BIN/$system_tool"
done
cat > "$SPEC_BIN/python-mock" <<'PYTHON'
#!/bin/sh
if [ "$1" = --version ]; then echo 'Python 3.12.1'; exit 0; fi
if [ "$1" = -m ] && [ "$2" = venv ]; then exit 0; fi
if [ -f "$1" ] && [ -n "$ARCHON_SETUP_PYTHON_SPECIFIER" ]; then
    exec "$REAL_PYTHON" -c 'import os,sys,collections; p=os.environ["MOCK_PYTHON_INFO"].split(","); v=collections.namedtuple("VersionInfo","major minor micro releaselevel serial"); sys.version_info=v(*([int(x) for x in p[:3]]+[p[3],int(p[4])])); probe=sys.argv[1]; sys.argv=[probe]; exec(compile(open(probe).read(),probe,"exec"))' "$1"
fi
exit 8
PYTHON
chmod +x "$SPEC_BIN/python-mock"
for versioned_python in python3.20 python3.19 python3.18 python3.17 python3.16 python3.15 python3.14 python3.13 python3.12 python3.11 python3.10 python3; do
    ln -s "$SPEC_BIN/python-mock" "$SPEC_BIN/$versioned_python"
done
run_spec_case() {
    spec=$1
    mock_info=$2
    expected=$3
    TEST_CASE="Python specifier $spec with $mock_info"
    case_name=$(printf '%s' "$spec-$mock_info" | tr -c 'A-Za-z0-9' '_')
    case_target="$TEMP_ROOT/spec $case_name"
    case_venv="$case_target/.archon/tools/openbb-venv"
    case_metadata="$case_venv/lib/python3.12/site-packages/openbb-99.dist-info"
    mkdir -p "$case_metadata"
    printf 'home = fake-python\n' > "$case_venv/pyvenv.cfg"
    printf 'Metadata-Version: 2.1\nName: openbb\nRequires-Python: %s\n' "$spec" > "$case_metadata/METADATA"
    printf 'preserve\n' > "$case_venv/sentinel"
    case_output=$(PATH="$SPEC_BIN:$MOCK_SYSTEM_BIN" REAL_PYTHON="$REAL_PYTHON" MOCK_PYTHON_INFO="$mock_info" \
        "$REPO_ROOT/scripts/setup-trading-tools.sh" --target "$case_target" --skip-tradingview 2>&1 || true)
    if printf '%s\n' "$case_output" | grep -F -- '-m venv --clear ' >/dev/null; then case_status=accepted; else case_status=rejected; fi
    if [ "$case_status" != "$expected" ]; then
        printf 'specifier case expected %s, got %s: %s / %s\n%s\n' "$expected" "$case_status" "$spec" "$mock_info" "$case_output" >&2
        exit 1
    fi
    if [ "$expected" = accepted ]; then
        printf '%s\n' "$case_output" | grep -F -- '-m venv --clear ' >/dev/null
    else
        printf '%s\n' "$case_output" | grep -F "specifier '$spec'" >/dev/null
        if [ "$spec" = '!===3.12' ]; then
            printf '%s\n' "$case_output" | grep -F 'unknown operator !=== (treated as unsatisfied)' >/dev/null
        fi
    fi
    test "$(cat "$case_venv/sentinel")" = preserve
}
run_spec_case '>=3.10,<3.13' '3,12,0,final,0' accepted
run_spec_case '>=3.10,<3.13' '3,13,0,final,0' rejected
run_spec_case '>=3.11.17' '3,11,0,final,0' rejected
run_spec_case '>=3.11.17' '3,11,17,final,0' accepted
run_spec_case '!=3.12.*' '3,12,4,final,0' rejected
run_spec_case '>=3.13' '3,13,0,candidate,1' rejected
run_spec_case '>=3.13rc1' '3,13,0,candidate,2' accepted
run_spec_case '>=3.13rc1' '3,13,0,final,0' accepted
run_spec_case '>=3.13rc1' '3,13,0,beta,1' rejected
run_spec_case '>=3.13rc1' '3,12,9,final,0' rejected
run_spec_case '>=3.13.0b2' '3,13,0,beta,1' rejected
run_spec_case '<3.14a1' '3,14,0,alpha,1' rejected
run_spec_case '!===3.12' '3,13,0,final,0' rejected

FALLBACK_BIN="$TEMP_ROOT/versioned-only interpreters"
mkdir -p "$FALLBACK_BIN"
for versioned_python in python3.20 python3.19 python3.18 python3.17 python3.16 python3.15 python3.14 python3.13 python3.12 python3.11 python3.10; do
    ln -s "$SPEC_BIN/python-mock" "$FALLBACK_BIN/$versioned_python"
done
UNSAT_DEFAULT_BIN="$TEMP_ROOT/unsatisfied default interpreter"
cp -R "$FALLBACK_BIN" "$UNSAT_DEFAULT_BIN"
cat > "$UNSAT_DEFAULT_BIN/python3" <<'PYTHON'
#!/bin/sh
if [ -f "$1" ] && [ -n "$ARCHON_SETUP_PYTHON_SPECIFIER" ]; then exit 1; fi
exit 8
PYTHON
chmod +x "$UNSAT_DEFAULT_BIN/python3"
run_python_order_case() {
    order_path=$1
    expected_interpreter=$2
    TEST_CASE="Python interpreter order prefers $expected_interpreter"
    order_target="$TEMP_ROOT/python order $expected_interpreter"
    order_venv="$order_target/.archon/tools/openbb-venv"
    mkdir -p "$order_venv"
    printf 'home = fake-python\n' > "$order_venv/pyvenv.cfg"
    order_metadata="$order_venv/lib/python3.12/site-packages/openbb-99.dist-info"
    mkdir -p "$order_metadata"
    printf 'Metadata-Version: 2.1\nName: openbb\nRequires-Python: >=3.10,<3.14\n' > "$order_metadata/METADATA"
    order_output=$(PATH="$order_path:$MOCK_SYSTEM_BIN" REAL_PYTHON="$REAL_PYTHON" \
        MOCK_PYTHON_INFO='3,12,0,final,0' \
        "$REPO_ROOT/scripts/setup-trading-tools.sh" --target "$order_target" --skip-tradingview 2>&1 || true)
    printf '%s\n' "$order_output" | grep -F "OpenBB Python: $expected_interpreter (Python 3.12.1)" >/dev/null
}
run_python_order_case "$SPEC_BIN" python3
run_python_order_case "$FALLBACK_BIN" python3.20
run_python_order_case "$UNSAT_DEFAULT_BIN" python3.20

cat > "$FAKE_BIN/git" <<'EOF'
#!/bin/sh
if [ "$1" = clone ]; then
    DEST="$3"
    mkdir -p "$DEST/src/cli" "$DEST/scripts"
    touch "$DEST/src/server.js" "$DEST/src/cli/index.js"
fi
EOF
cat > "$FAKE_BIN/npm" <<'EOF'
#!/bin/sh
if [ "$1" = install ]; then
    printf '%s\n' "$PWD" "$*" > "$FAKE_NPM_LOG"
else
    echo 'npm 10.0.0'
fi
exit 0
EOF
cat > "$FAKE_BIN/node" <<'EOF'
#!/bin/sh
if [ "$1" = - ]; then printf '{}\n' > "$2"; fi
EOF
chmod +x "$FAKE_BIN/git" "$FAKE_BIN/npm" "$FAKE_BIN/node"
TV_TARGET="$TEMP_ROOT/tv target"
TEST_CASE='TradingView install with spaced target'
mkdir -p "$TV_TARGET"
TV_TARGET=$(cd "$TV_TARGET" && pwd)
FAKE_NPM_LOG="$TEMP_ROOT/npm invocation.log" PATH="$FAKE_BIN:$PATH" "$REPO_ROOT/scripts/setup-trading-tools.sh" \
    --target "$TV_TARGET" --skip-openbb
TV_DIR="$TV_TARGET/.archon/tools/tradingview-mcp"
NPM_CWD=$(sed -n '1p' "$TEMP_ROOT/npm invocation.log")
NPM_ARGS=$(sed -n '2p' "$TEMP_ROOT/npm invocation.log")
if [ "$NPM_CWD" != "$TV_DIR" ] || [ "$NPM_ARGS" != 'install --omit=dev' ]; then
    printf 'npm invocation expected cwd=%s args=%s, got cwd=%s args=%s\n' "$TV_DIR" 'install --omit=dev' "$NPM_CWD" "$NPM_ARGS" >&2
    exit 1
fi
test -f "$TV_TARGET/scripts/start-tradingview-cdp.sh"
test -f "$TV_TARGET/scripts/start-tradingview-cdp.ps1"
grep -F 'launch_tv_debug_windows.ps1' "$TV_TARGET/scripts/start-tradingview-cdp.ps1" >/dev/null
grep -F '9222' "$TV_TARGET/scripts/start-tradingview-cdp.ps1" >/dev/null
TEST_CASE='missing external TradingView launcher refusal'
if tv_output=$("$TV_TARGET/scripts/start-tradingview-cdp.sh" 9222 2>&1); then
    echo 'expected missing external TradingView launcher to fail' >&2
    exit 1
fi
printf '%s\n' "$tv_output" | grep -F 'Install the external TradingView launcher project' >/dev/null
echo 'test result: setup-trading-tools shell checks passed'
