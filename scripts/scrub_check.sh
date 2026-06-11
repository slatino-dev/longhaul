#!/usr/bin/env bash
# scrub_check.sh — fail (exit 1) if any tracked file contains sensitive patterns.
# Wire this into CI before merging.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

PATTERNS=(
    'redacted-host'
    '100\.64\.[0-9]+\.[0-9]+'
    'ts\.net'
    'redacted-mesh'
    'redacted-key'
    'sk-[A-Za-z0-9]{20,}'
)

FOUND=0

for pattern in "${PATTERNS[@]}"; do
    # Search all files tracked by git (or all files if not in a git repo yet)
    if git -C "$REPO_ROOT" rev-parse --is-inside-work-tree &>/dev/null 2>&1; then
        FILES=$(git -C "$REPO_ROOT" ls-files)
    else
        FILES=$(find "$REPO_ROOT" -type f -not -path '*/.git/*')
    fi

    while IFS= read -r file; do
        abs="$REPO_ROOT/$file"
        [ -f "$abs" ] || continue
        matches=$(grep -nEo "$pattern" "$abs" 2>/dev/null || true)
        if [ -n "$matches" ]; then
            echo "SCRUB FAIL: pattern '$pattern' matched in $file:"
            grep -nE "$pattern" "$abs" | head -5
            FOUND=1
        fi
    done <<< "$FILES"
done

if [ "$FOUND" -ne 0 ]; then
    echo ""
    echo "scrub_check: FAILED — sensitive pattern(s) detected. Remove them before committing."
    exit 1
fi

echo "scrub_check: PASSED — no sensitive patterns detected."
