#!/usr/bin/env bash
# Generate Python protobuf modules from proto/ (the single source of truth).
#
# Output: services/python/generated/<package path>_pb2.py with __init__.py
# files created so the directory is importable as a package. Generated code is
# never committed (see .gitignore and scripts/clean-artifacts.sh).
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PROTO_DIR="$ROOT_DIR/proto"
OUT_DIR="$ROOT_DIR/services/python/generated"

if ! command -v python >/dev/null 2>&1 && ! command -v python3 >/dev/null 2>&1; then
    echo "error: python is required to generate protobuf modules" >&2
    exit 1
fi
PYTHON_BIN="$(command -v python || command -v python3)"

rm -rf "$OUT_DIR"
mkdir -p "$OUT_DIR"

# Collect every .proto relative to the include root.
mapfile -t PROTOS < <(cd "$PROTO_DIR" && find . -name '*.proto' | sed 's|^\./||' | sort)
if [ "${#PROTOS[@]}" -eq 0 ]; then
    echo "error: no .proto files found under $PROTO_DIR" >&2
    exit 1
fi

"$PYTHON_BIN" -m grpc_tools.protoc \
    --proto_path="$PROTO_DIR" \
    --python_out="$OUT_DIR" \
    "${PROTOS[@]/#/$PROTO_DIR/}"

# Make every generated directory an importable package.
find "$OUT_DIR" -type d -exec sh -c 'touch "$1/__init__.py"' _ {} \;

# protoc emits imports relative to the include root (``from common.v1 import
# common_pb2``), while workers import generated modules as ``generated.*``.
# Rewrite the imports so both namespaces agree and descriptors are registered
# exactly once.
"$PYTHON_BIN" - "$OUT_DIR" <<'PY'
import pathlib
import re
import sys

out_dir = pathlib.Path(sys.argv[1])
pattern = re.compile(r"^from ((?:[A-Za-z_][A-Za-z0-9_]*\.)+v[0-9]+) import ", re.MULTILINE)
for path in out_dir.rglob("*_pb2*.py"):
    text = path.read_text(encoding="utf-8")
    rewritten = pattern.sub(r"from generated.\1 import ", text)
    if rewritten != text:
        path.write_text(rewritten, encoding="utf-8", newline="\n")
PY

echo "generated $(printf '%s\n' "${PROTOS[@]}" | wc -l) proto module(s) into $OUT_DIR"
