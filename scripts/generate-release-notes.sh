#!/usr/bin/env bash
set -euo pipefail

# Generate release notes into a file.
# Usage: TAG=v1.2.3 PREV_TAG=v1.2.2 scripts/generate-release-notes.sh OUTPUT.md

OUT_FILE=${1:-RELEASE_BODY.md}
TAG=${TAG:-${GITHUB_REF_NAME:-}}
PREV_TAG=${PREV_TAG:-}

if [[ -z "$TAG" ]]; then
  echo "TAG env not set" >&2
  exit 1
fi

# Ensure we have full history and tags available
git fetch --tags --force --prune >/dev/null 2>&1 || true

# If PREV_TAG not provided, try to infer previous tag by version sort
if [[ -z "${PREV_TAG}" ]]; then
  PREV_TAG=$(git tag --sort=-version:refname | awk -v cur="$TAG" '$0!=cur {print; exit}') || true
fi

RANGE=""
if [[ -n "${PREV_TAG}" ]]; then
  RANGE="${PREV_TAG}..HEAD"
else
  RANGE="--max-parents=0 HEAD..HEAD" # will fall back to all commits below
fi

COMMITS=$(git log --pretty=format:'%s|%h' ${PREV_TAG:+${PREV_TAG}..}HEAD || true)

features=()
fixes=()
perf=()
others=()

while IFS= read -r line; do
  [[ -z "$line" ]] && continue
  subject="${line%%|*}"
  hash="${line##*|}"
  lc=$(echo "$subject" | tr 'A-Z' 'a-z')
  if [[ $lc == feat:* || $lc == feat\(* ]]; then
    features+=("- ${subject} (${hash})")
  elif [[ $lc == fix:* || $lc == fix\(* ]]; then
    fixes+=("- ${subject} (${hash})")
  elif [[ $lc == perf:* || $lc == perf\(* ]]; then
    perf+=("- ${subject} (${hash})")
  else
    others+=("- ${subject} (${hash})")
  fi
done <<< "$COMMITS"

{
  echo "# U-Download ${TAG}"
  echo
  if [[ -n "$PREV_TAG" ]]; then
    echo "Compare: https://github.com/okwareddevnet/u-download/compare/${PREV_TAG}...${TAG}"
    echo
  fi

  if (( ${#features[@]} )); then
    echo "## What's New"
    printf '%s\n' "${features[@]}"
    echo
  fi

  if (( ${#fixes[@]} )); then
    echo "## Fixes"
    printf '%s\n' "${fixes[@]}"
    echo
  fi

  if (( ${#perf[@]} )); then
    echo "## Performance"
    printf '%s\n' "${perf[@]}"
    echo
  fi

  if (( ${#others[@]} )); then
    echo "## Other Changes"
    printf '%s\n' "${others[@]}"
    echo
  fi

  echo "## Packages"
  echo "- Linux: .deb and .rpm installers"
  echo "- Windows: NSIS .exe installer"
  echo "- macOS: .dmg (Intel and Apple Silicon)"
  echo
  echo "Assets are attached to this release."
} > "$OUT_FILE"

echo "Generated release notes at $OUT_FILE"

