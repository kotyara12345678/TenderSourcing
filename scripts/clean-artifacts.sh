#!/usr/bin/env bash
# ============================================================================
# clean-artifacts.sh — removes ONLY known generated build artifacts.
#
# Safety rules enforced here:
#   * operates exclusively inside the repository root (detected via git);
#   * never touches source code, .env*, docs, migrations, uploads/, data/,
#     storage/, node_modules is NOT removed (deps reinstall is expensive),
#   * supports --dry-run to only print what would be removed,
#   * no unconditional `rm -rf` over arbitrary paths — every path is matched
#     against an explicit allowlist of generated artifacts.
#
# Usage:
#   scripts/clean-artifacts.sh            # remove artifacts
#   scripts/clean-artifacts.sh --dry-run  # print only
# ============================================================================
set -euo pipefail

DRY_RUN=0
[[ "${1:-}" == "--dry-run" ]] && DRY_RUN=1
[[ "${1:-}" == "-h" || "${1:-}" == "--help" ]] && { sed -n '2,17p' "$0" | sed 's/^# \{0,1\}//'; exit 0; }

# --- Locate repository root (must be inside a git work tree) ---------------
if ! ROOT="$(git rev-parse --show-toplevel 2>/dev/null)"; then
  echo "ERROR: not inside a git repository — refusing to clean anything." >&2
  exit 1
fi
cd "$ROOT"

# --- Allowlist: exact relative paths of generated artifacts ----------------
ARTIFACT_DIRS=(
  "target"                          # Rust build artifacts
  "services/rust/target"            # workspace target (if nested)
  ".pytest_cache"
  ".ruff_cache"
  ".mypy_cache"
  "frontend/dist"
  "dist"
  "build"
  "tmp"
  "coverage"
  "htmlcov"
  "services/python/document_worker/.pytest_cache"
  "services/python/ai_worker/.pytest_cache"
  "services/python/report_worker/.pytest_cache"
)

# --- Allowlist: generated file patterns ------------------------------------
ARTIFACT_GLOBS=(
  "__pycache__"
  "*.pyc"
  "*.pyo"
  ".DS_Store"
)

# --- Never-remove list (explicitly checked before any deletion) ------------
PROTECTED_PREFIXES=(
  ".env"
  "docs"
  "migrations"
  "uploads"
  "data"
  "storage"
  "proto"
  "scripts"
  ".git"
  "node_modules"
)

is_protected() {
  local rel="$1"
  for p in "${PROTECTED_PREFIXES[@]}"; do
    [[ "$rel" == "$p" || "$rel" == "$p"/* ]] && return 0
  done
  return 1
}

removed=0
skipped=0

remove_path() {
  local rel="$1"
  # normalize: strip leading ./ and trailing /
  rel="${rel#./}"; rel="${rel%/}"
  [[ -z "$rel" ]] && return
  if is_protected "$rel"; then
    echo "SKIP (protected): $rel"
    skipped=$((skipped + 1))
    return
  fi
  # must stay inside repo root
  case "$rel" in
    /*|*".."*) echo "SKIP (outside root): $rel"; skipped=$((skipped + 1)); return ;;
  esac
  if [[ ! -e "$rel" ]]; then return; fi
  if [[ $DRY_RUN -eq 1 ]]; then
    echo "WOULD REMOVE: $rel"
  else
    echo "REMOVING:     $rel"
    rm -rf -- "$rel"
  fi
  removed=$((removed + 1))
}

echo "== clean-artifacts (root: $ROOT, dry-run: $DRY_RUN) =="

for d in "${ARTIFACT_DIRS[@]}"; do
  [[ -d "$d" ]] && remove_path "$d"
done

# __pycache__ and other pattern-based artifacts anywhere in the tree (depth-limited)
while IFS= read -r -d '' p; do
  remove_path "${p#./}"
done < <(find . -path ./node_modules -prune -o -path ./.git -prune -o \
              \( -name "__pycache__" -o -name ".pytest_cache" -o -name ".ruff_cache" \
                 -o -name "*.pyc" \) -print0 2>/dev/null)

echo "== done: $removed path(s) $([[ $DRY_RUN -eq 1 ]] && echo 'would be ' || echo '')removed, $skipped protected/skipped =="
