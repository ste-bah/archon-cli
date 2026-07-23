#!/usr/bin/env bash

set -uo pipefail

if [[ "$#" -ne 1 ]]; then
  echo "usage: list-cargo-tests.sh <output-file>" >&2
  exit 2
fi

OUTPUT_FILE="$1"
OUTPUT_DIR="$(dirname -- "$OUTPUT_FILE")"
TMPDIR_RUN="$(mktemp -d)"
OUTPUT_TMP="$(mktemp "$OUTPUT_DIR/.cargo-test-list.XXXXXX")"
EXECUTABLES="$TMPDIR_RUN/executables.tsv"
trap 'rm -rf "$TMPDIR_RUN"; rm -f "$OUTPUT_TMP"' EXIT

if ! cargo metadata --no-deps --format-version 1 > "$TMPDIR_RUN/metadata.json"; then
  echo "[list-cargo-tests] cargo metadata failed" >&2
  exit 1
fi

if ! python3 - "$TMPDIR_RUN/metadata.json" > "$TMPDIR_RUN/packages.tsv" <<'PY'
import json
import sys

with open(sys.argv[1], encoding="utf-8") as handle:
    metadata = json.load(handle)

members = set(metadata["workspace_members"])
for package in sorted(metadata["packages"], key=lambda item: item["name"]):
    if package["id"] in members:
        print(package["id"], package["name"], sep="\t")
PY
then
  echo "[list-cargo-tests] workspace package parsing failed" >&2
  exit 1
fi

if [[ ! -s "$TMPDIR_RUN/packages.tsv" ]]; then
  echo "[list-cargo-tests] workspace contains no packages" >&2
  exit 1
fi

BUILD_JSON="$TMPDIR_RUN/build.json"
BUILD_ERR="$TMPDIR_RUN/build.err"
if timeout 1800 cargo test --workspace --jobs 1 --no-fail-fast \
    --no-run --message-format=json > "$BUILD_JSON" 2> "$BUILD_ERR"; then
  :
else
  build_rc=$?
  cat "$BUILD_ERR" >&2
  if [[ "$build_rc" -eq 124 ]]; then
    echo "[list-cargo-tests] workspace test build timed out" >&2
  else
    printf '[list-cargo-tests] workspace test build failed (exit %s)\n' "$build_rc" >&2
  fi
  exit "$build_rc"
fi

if ! python3 - "$BUILD_JSON" "$TMPDIR_RUN/packages.tsv" > "$EXECUTABLES" <<'PY'
import json
import sys

build_path, packages_path = sys.argv[1:]
with open(packages_path, encoding="utf-8") as handle:
    package_names = dict(line.rstrip("\n").split("\t", 1) for line in handle)

with open(build_path, encoding="utf-8") as handle:
    for line in handle:
        message = json.loads(line)
        if message.get("reason") != "compiler-artifact":
            continue
        package_name = package_names.get(message.get("package_id"))
        target = message.get("target", {})
        executable = message.get("executable")
        if package_name and target.get("test") and message.get("profile", {}).get("test") and executable:
            kind = ",".join(target.get("kind", []))
            if not kind:
                raise ValueError(f'test target {target.get("name")} has no kind')
            print(package_name, kind, target["name"], executable, sep="\t")
PY
then
  echo "[list-cargo-tests] Cargo artifact parsing failed" >&2
  exit 1
fi

LC_ALL=C sort -u -o "$EXECUTABLES" "$EXECUTABLES"
if [[ ! -s "$EXECUTABLES" ]]; then
  echo "[list-cargo-tests] workspace contains no test executables" >&2
  exit 1
fi

: > "$OUTPUT_TMP"
index=0
while IFS=$'\t' read -r package kind target executable; do
  TARGET_LOG="$TMPDIR_RUN/list-$index.out"
  TARGET_ERR="$TMPDIR_RUN/list-$index.err"
  index=$((index + 1))
  if timeout 600 "$executable" --list --format=terse > "$TARGET_LOG" 2> "$TARGET_ERR"; then
    :
  else
    target_rc=$?
    cat "$TARGET_ERR" >&2
    if [[ "$target_rc" -eq 124 ]]; then
      printf '[list-cargo-tests] %s::%s::%s discovery timed out\n' "$package" "$kind" "$target" >&2
    else
      printf '[list-cargo-tests] %s::%s::%s discovery failed (exit %s)\n' \
        "$package" "$kind" "$target" "$target_rc" >&2
    fi
    exit "$target_rc"
  fi

  LC_ALL=C awk -v package="$package" -v kind="$kind" -v target="$target" '
    /: test$/ {
      sub(/: test$/, "")
      if ($0 !~ /\.rs - .* \(line [0-9]+\)$/) {
        print package "::" kind "::" target "::" $0
      }
    }
  ' "$TARGET_LOG" >> "$OUTPUT_TMP"
done < "$EXECUTABLES"

LC_ALL=C sort -u -o "$OUTPUT_TMP" "$OUTPUT_TMP"
if [[ ! -s "$OUTPUT_TMP" ]]; then
  echo "[list-cargo-tests] no unit tests discovered" >&2
  exit 1
fi

mv "$OUTPUT_TMP" "$OUTPUT_FILE"
