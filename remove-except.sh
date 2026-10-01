#!/usr/bin/env bash
# remove-except.sh
# Remove everything under the current directory EXCEPT items matching the given patterns.
#
# Usage:
#   remove-except.sh [--dry-run] [--force] [--] <pattern> [<pattern>...]
#
# Options:
#   --dry-run   Show what would be deleted without removing anything.
#   --force     Skip the confirmation prompt.
#   --          Treat all following arguments as patterns, even if they start with '-'.
#
# Patterns are interpreted relative to the current directory (or as absolute paths).
# Wildcards (* ? [) are supported. Example:
#   remove-except.sh 'dist' '*.md'

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=remove-except-common.sh
source "$SCRIPT_DIR/remove-except-common.sh"

# ---------------------------------------------------------------------------
# Argument parsing
# ---------------------------------------------------------------------------
dry_run=0
force=0
parse_options=1
patterns=()

for arg in "$@"; do
    if [[ "$parse_options" -eq 0 ]]; then
        patterns+=("$arg")
        continue
    fi

    case "$arg" in
        --dry-run) dry_run=1 ;;
        --force)   force=1 ;;
        --)        parse_options=0 ;;
        -*)
            echo "Unknown option: $arg" >&2
            echo "Usage: remove-except.sh [--dry-run] [--force] [--] <pattern> [<pattern>...]" >&2
            exit 1
            ;;
        *) patterns+=("$arg") ;;
    esac
done

if [[ ${#patterns[@]} -eq 0 ]]; then
    echo "Usage: remove-except.sh [--dry-run] [--force] [--] <pattern> [<pattern>...]" >&2
    exit 1
fi

# ---------------------------------------------------------------------------
# Build plan
# ---------------------------------------------------------------------------
rex_get_remove_except_plan "$(pwd)" "${patterns[@]}"

if [[ "$REX_PLAN_DIRECT_MATCH_COUNT" -eq 0 ]]; then
    echo "WARNING: No items matched the keep patterns. Everything under the current directory would be removed." >&2
fi

# ---------------------------------------------------------------------------
# Display table
# ---------------------------------------------------------------------------
if [[ ${#REX_PLAN_DELETE_ITEMS[@]} -eq 0 ]]; then
    echo "(Nothing to delete.)"
else
    printf '%-12s  %-40s  %s\n' "ItemType" "RelativePath" "FullName"
    printf '%-12s  %-40s  %s\n' "--------" "------------" "--------"
    for entry in "${REX_PLAN_DELETE_ITEMS[@]}"; do
        IFS=$'\t' read -r itype relpath fullpath <<< "$entry"
        printf '%-12s  %-40s  %s\n' "$itype" "$relpath" "$fullpath"
    done
fi

# ---------------------------------------------------------------------------
# DryRun exit
# ---------------------------------------------------------------------------
if [[ "$dry_run" -eq 1 ]]; then
    exit 0
fi

if [[ ${#REX_PLAN_DELETE_ROOTS[@]} -eq 0 ]]; then
    exit 0
fi

# ---------------------------------------------------------------------------
# Confirmation
# ---------------------------------------------------------------------------
if [[ "$force" -eq 0 ]]; then
    echo ""
    printf 'Are you sure you want to delete the above items? [y/N] '
    read -r answer
    case "$answer" in
        [yY]|[yY][eE][sS]) ;;
        *)
            echo "Aborted."
            exit 0
            ;;
    esac
fi

# ---------------------------------------------------------------------------
# Delete
# ---------------------------------------------------------------------------
for entry in "${REX_PLAN_DELETE_ROOTS[@]}"; do
    IFS=$'\t' read -r _itype _relpath fullpath <<< "$entry"
    rm -rf -- "$fullpath"
done
