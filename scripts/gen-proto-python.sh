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

echo "generated $(printf '%s\n' "${PROTOS[@]}" | wc -l) proto module(s) into $OUT_DIR"
