#!/usr/bin/env bash
# test-remove-except.sh
# Preview what remove-except.sh would delete, without removing anything.
#
# Usage:
#   test-remove-except.sh <pattern> [<pattern>...]

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=remove-except-common.sh
source "$SCRIPT_DIR/remove-except-common.sh"

if [[ $# -eq 0 ]]; then
    echo "Usage: test-remove-except.sh <pattern> [<pattern>...]" >&2
    exit 1
fi

rex_get_remove_except_plan "$(pwd)" "$@"

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
