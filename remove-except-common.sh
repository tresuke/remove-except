#!/usr/bin/env bash
# remove-except-common.sh
# Common library for remove-except.sh and test-remove-except.sh.
# Requires Bash 4+ (associative arrays).

set -euo pipefail

# ---------------------------------------------------------------------------
# Internal helpers (_rex_ prefix)
# ---------------------------------------------------------------------------

# Normalize a path:
#   - Collapse repeated slashes
#   - Optionally trim trailing slash (pass "trim" as second arg)
# Returns the normalized path on stdout.
_rex_normalize_path() {
    local path="$1"
    local trim="${2:-}"

    # Collapse consecutive slashes without spawning external commands.
    local normalized="$path"
    while [[ "$normalized" == *//* ]]; do
        normalized="${normalized//\/\//\/}"
    done

    if [[ "$trim" == "trim" ]]; then
        # Remove trailing slash unless the path is exactly "/"
        if [[ "$normalized" != "/" ]]; then
            normalized="${normalized%/}"
        fi
    fi

    printf '%s' "$normalized"
}

# Returns 0 if the pattern contains wildcard characters (* ? [), 1 otherwise.
_rex_has_wildcard() {
    local pattern="$1"
    [[ "$pattern" == *'*'* || "$pattern" == *'?'* || "$pattern" == *'['* ]]
}

# Returns 0 if the pattern contains a parent-traversal segment (..), 1 otherwise.
_rex_has_parent_traversal() {
    local pattern="$1"
    [[ "$pattern" =~ (^|[/])\.\.(|[/]) ]]
}

# Returns 0 if candidate is equal to root or is inside root.
_rex_is_within_root() {
    local root="$1"
    local candidate="$2"

    if [[ "$candidate" == "$root" ]]; then
        return 0
    fi

    # Get the relative path from root to candidate.
    local rel
    rel=$(realpath --no-symlinks --relative-to="$root" "$candidate" 2>/dev/null) || return 1

    # If the relative path starts with ".." it's outside the root.
    if [[ "$rel" == ".." || "$rel" == ../* ]]; then
        return 1
    fi
    # If it is absolute it escaped the root somehow.
    if [[ "$rel" == /* ]]; then
        return 1
    fi
    return 0
}

# Concatenate root and relative pattern, then normalize.
_rex_join_abs_pattern() {
    local root="$1"
    local rel_pattern="$2"

    if [[ -z "$rel_pattern" ]]; then
        printf '%s' "$root"
        return
    fi

    _rex_normalize_path "$root/$rel_pattern"
}

# Extract the non-wildcard directory prefix of a pattern.
# Prints the prefix to stdout and returns 0.
# Prints nothing and returns 1 when no deterministic prefix exists (wildcard in dir part).
_rex_get_dir_prefix() {
    local pattern="$1"

    if [[ -z "$pattern" ]]; then
        printf ''
        return 0
    fi

    local normalized
    normalized=$(_rex_normalize_path "$pattern" "trim")

    # Pattern ends with "/*"  →  return the part before "/*"
    if [[ "$normalized" == */* ]]; then
        local last_seg="${normalized##*/}"
        local dir_part="${normalized%/*}"
        if [[ "$last_seg" == "*" ]]; then
            printf '%s' "$(_rex_normalize_path "$dir_part" "trim")"
            return 0
        fi
    fi

    # No wildcards  →  the whole pattern is the prefix
    if ! _rex_has_wildcard "$normalized"; then
        printf '%s' "$normalized"
        return 0
    fi

    # Wildcard in path  →  no deterministic prefix
    return 1
}

# ---------------------------------------------------------------------------
# Pattern spec storage
# Global arrays indexed by integer (0-based).
# ---------------------------------------------------------------------------
# _REX_PS_ORIG[]           Original pattern string
# _REX_PS_REL_PAT[]        Relative pattern (empty string = root itself; "__NULL__" = not set)
# _REX_PS_ABS_PAT[]        Absolute pattern
# _REX_PS_REL_PREFIX[]     Relative prefix  ("__NULL__" = not set / wildcard in dir)
# _REX_PS_ABS_PREFIX[]     Absolute prefix  ("__NULL__" = not set)
_REX_PS_ORIG=()
_REX_PS_REL_PAT=()
_REX_PS_ABS_PAT=()
_REX_PS_REL_PREFIX=()
_REX_PS_ABS_PREFIX=()

# Compile a pattern and store it at index IDX.
# Usage: _rex_new_pattern_spec <idx> <pattern> <root>
_rex_new_pattern_spec() {
    local idx="$1"
    local pattern="$2"
    local root="$3"

    # Trim whitespace
    local raw
    raw=$(printf '%s' "$pattern" | sed 's/^[[:space:]]*//;s/[[:space:]]*$//')
    if [[ -z "$raw" ]]; then
        echo "ERROR: Pattern must not be empty." >&2
        return 1
    fi

    if _rex_has_parent_traversal "$raw"; then
        echo "ERROR: Parent directory traversal is not allowed in pattern: $pattern" >&2
        return 1
    fi

    local normalized_root
    normalized_root=$(_rex_normalize_path "$root" "trim")

    local rel_pattern
    local abs_pattern

    if [[ "$raw" == /* ]]; then
        # Absolute pattern
        abs_pattern=$(_rex_normalize_path "$raw" "trim")
        rel_pattern="__NULL__"

        if ! _rex_has_wildcard "$abs_pattern"; then
            # Resolve to a canonical path (lexically, no symlink resolution required)
            local full_path
            full_path=$(realpath --no-symlinks "$abs_pattern" 2>/dev/null) || full_path=$abs_pattern
            full_path=$(_rex_normalize_path "$full_path" "trim")

            if ! _rex_is_within_root "$normalized_root" "$full_path"; then
                echo "ERROR: Absolute pattern is outside the current root: $pattern" >&2
                return 1
            fi

            abs_pattern="$full_path"

            if [[ "$full_path" == "$normalized_root" ]]; then
                rel_pattern=""
            else
                local rel
                rel=$(realpath --no-symlinks --relative-to="$normalized_root" "$full_path" 2>/dev/null) || rel="${full_path#$normalized_root/}"
                rel_pattern=$(_rex_normalize_path "$rel" "trim")
            fi
        elif [[ "$abs_pattern" == "$normalized_root"* ]]; then
            local rel_candidate="${abs_pattern#$normalized_root}"
            rel_candidate="${rel_candidate#/}"
            rel_pattern=$(_rex_normalize_path "$rel_candidate")
        fi
    else
        # Relative pattern — strip leading "./" segments
        local stripped
        stripped=$(printf '%s' "$raw" | sed 's|^\(\./\)*||')
        stripped=$(_rex_normalize_path "$stripped" "trim")
        # Remove any remaining leading slashes that appeared after normalization
        stripped="${stripped#/}"
        rel_pattern="$stripped"
        abs_pattern=$(_rex_join_abs_pattern "$normalized_root" "$rel_pattern")
    fi

    # Compute prefixes
    local rel_prefix
    local rel_prefix_null=0
    if [[ "$rel_pattern" == "__NULL__" ]]; then
        rel_prefix="__NULL__"
        rel_prefix_null=1
    else
        if rel_prefix=$(_rex_get_dir_prefix "$rel_pattern"); then
            : # success
        else
            rel_prefix="__NULL__"
            rel_prefix_null=1
        fi
    fi

    local abs_prefix
    local abs_prefix_null=0
    if abs_prefix=$(_rex_get_dir_prefix "$abs_pattern"); then
        : # success
    else
        abs_prefix="__NULL__"
        abs_prefix_null=1
    fi

    _REX_PS_ORIG[$idx]="$pattern"
    _REX_PS_REL_PAT[$idx]="$rel_pattern"
    _REX_PS_ABS_PAT[$idx]="$abs_pattern"
    _REX_PS_REL_PREFIX[$idx]="$rel_prefix"
    _REX_PS_ABS_PREFIX[$idx]="$abs_prefix"
}

# Returns 0 if the item (given by relative and absolute path) is matched
# by the pattern spec at index IDX.
# Usage: _rex_item_matches <relpath> <abspath> <idx>
_rex_item_matches() {
    local rel="$1"
    local abs="$2"
    local idx="$3"

    local rel_pat="${_REX_PS_REL_PAT[$idx]}"
    local abs_pat="${_REX_PS_ABS_PAT[$idx]}"
    local rel_prefix="${_REX_PS_REL_PREFIX[$idx]}"
    local abs_prefix="${_REX_PS_ABS_PREFIX[$idx]}"

    # relativeLike
    if [[ "$rel_pat" != "__NULL__" ]]; then
        # shellcheck disable=SC2254  # we intentionally use an unquoted glob pattern
        if [[ "$rel" == $rel_pat ]]; then
            return 0
        fi
    fi

    # absoluteLike
    if [[ "$abs_pat" != "__NULL__" ]]; then
        # shellcheck disable=SC2254
        if [[ "$abs" == $abs_pat ]]; then
            return 0
        fi
    fi

    # relativePrefix
    if [[ "$rel_prefix" != "__NULL__" ]]; then
        if [[ -z "$rel_prefix" || "$rel" == "$rel_prefix" || "$rel" == "$rel_prefix"/* ]]; then
            return 0
        fi
    fi

    # absolutePrefix
    if [[ "$abs_prefix" != "__NULL__" ]]; then
        if [[ "$abs" == "$abs_prefix" || "$abs" == "$abs_prefix"/* ]]; then
            return 0
        fi
    fi

    return 1
}

# ---------------------------------------------------------------------------
# Plan output globals
# ---------------------------------------------------------------------------
REX_PLAN_ROOT=""
REX_PLAN_PATSPEC_COUNT=0
REX_PLAN_DIRECT_MATCH_COUNT=0
# Each entry is TAB-separated: type<TAB>relpath<TAB>fullpath
# "type" is "Directory" or "File"
REX_PLAN_DELETE_ITEMS=()
REX_PLAN_DELETE_ROOTS=()

# ---------------------------------------------------------------------------
# Public: rex_get_remove_except_plan <root> <pattern> [<pattern>...]
# ---------------------------------------------------------------------------
rex_get_remove_except_plan() {
    local root="$1"
    shift
    local patterns=("$@")

    if [[ ${#patterns[@]} -eq 0 ]]; then
        echo "ERROR: At least one pattern is required." >&2
        return 1
    fi

    # Reset globals
    _REX_PS_ORIG=()
    _REX_PS_REL_PAT=()
    _REX_PS_ABS_PAT=()
    _REX_PS_REL_PREFIX=()
    _REX_PS_ABS_PREFIX=()
    REX_PLAN_DELETE_ITEMS=()
    REX_PLAN_DELETE_ROOTS=()
    REX_PLAN_DIRECT_MATCH_COUNT=0

    local normalized_root
    normalized_root=$(_rex_normalize_path "$(realpath --no-symlinks "$root")" "trim")
    REX_PLAN_ROOT="$normalized_root"

    # Compile pattern specs
    local i=0
    for pat in "${patterns[@]}"; do
        _rex_new_pattern_spec "$i" "$pat" "$normalized_root"
        (( i++ )) || true
    done
    REX_PLAN_PATSPEC_COUNT=$i

    # Build item metadata and keep ancestors as each direct match is found.
    # keep_paths is an associative array: key=abspath, value=1
    declare -A keep_paths=()

    local -a item_abs=()
    local -a item_rel=()
    local -a item_type=()   # "File" or "Directory"
    local j=0
    local row
    while IFS= read -r -d '' row; do
        local kind rel_item norm_abs
        IFS=$'\t' read -r kind rel_item norm_abs <<< "$row"

        local itype="File"
        if [[ "$kind" == "d" ]]; then
            itype="Directory"
        fi

        item_abs[$j]="$norm_abs"
        item_rel[$j]="$rel_item"
        item_type[$j]="$itype"

        # Check against all pattern specs
        local matched=0
        local k
        for (( k=0; k < REX_PLAN_PATSPEC_COUNT; k++ )); do
            if _rex_item_matches "$rel_item" "$norm_abs" "$k"; then
                matched=1
                break
            fi
        done
        if [[ "$matched" == "1" ]]; then
            REX_PLAN_DIRECT_MATCH_COUNT=$(( REX_PLAN_DIRECT_MATCH_COUNT + 1 ))

            local cur="$norm_abs"
            while true; do
                if [[ -v "keep_paths[$cur]" ]]; then
                    break
                fi
                keep_paths["$cur"]=1

                if [[ "$cur" == "$normalized_root" ]]; then
                    break
                fi

                local parent="${cur%/*}"
                if [[ -z "$parent" ]]; then
                    parent="/"
                fi
                cur="$parent"
            done
        fi

        (( j++ )) || true
    done < <(find "$normalized_root" -mindepth 1 -printf '%y\t%P\t%p\0')

    local total_items=$j
    local d

    # DeleteItems: retain the existing relative-path output order without sorting kept items.
    local -a delete_abs=()
    local -a delete_rel=()
    local -a delete_type=()

    for (( d=0; d < total_items; d++ )); do
        if [[ ! -v "keep_paths[${item_abs[$d]}]" ]]; then
            delete_abs+=("${item_abs[$d]}")
            delete_rel+=("${item_rel[$d]}")
            delete_type+=("${item_type[$d]}")
            REX_PLAN_DELETE_ITEMS+=("${item_type[$d]}	${item_rel[$d]}	${item_abs[$d]}")
        fi
    done

    if [[ ${#REX_PLAN_DELETE_ITEMS[@]} -gt 0 ]]; then
        local -a sorted_delete_items=()
        while IFS= read -r -d '' line; do
            sorted_delete_items+=("$line")
        done < <(printf '%s\0' "${REX_PLAN_DELETE_ITEMS[@]}" | sort -z -t$'\t' -k2,2)
        REX_PLAN_DELETE_ITEMS=("${sorted_delete_items[@]}")
    fi

    # Build lookup maps for O(1) parent checks in DeleteRoots calculation.
    declare -A delete_lookup=()
    declare -A delete_type_map=()
    for abs_d in "${delete_abs[@]}"; do
        delete_lookup["$abs_d"]=1
    done
    local idx
    for (( idx=0; idx < ${#delete_abs[@]}; idx++ )); do
        delete_type_map["${delete_abs[$idx]}"]="${delete_type[$idx]}"
    done

    # DeleteRoots: delete items whose parent (up to root) is not also a delete directory
    local nd=${#delete_abs[@]}
    local r
    for (( r=0; r < nd; r++ )); do
        local abs_r="${delete_abs[$r]}"
        local skip=0
        local parent_r="${abs_r%/*}"

        while true; do
            if [[ -z "$parent_r" ]]; then
                parent_r="/"
            fi

            if [[ -v "delete_lookup[$parent_r]" ]]; then
                # Parent is in delete set — skip when the parent is a directory.
                if [[ "${delete_type_map[$parent_r]}" == "Directory" ]]; then
                    skip=1
                    break
                fi
            fi

            if [[ "$parent_r" == "$normalized_root" ]]; then
                break
            fi

            parent_r="${parent_r%/*}"
        done

        if [[ "$skip" == "0" ]]; then
            REX_PLAN_DELETE_ROOTS+=("${delete_type[$r]}	${delete_rel[$r]}	${delete_abs[$r]}")
        fi
    done

    # Sort DeleteRoots: deepest first (most slashes), then alphabetically
    if [[ ${#REX_PLAN_DELETE_ROOTS[@]} -gt 0 ]]; then
        local -a sorted_roots=()
        while IFS= read -r line; do
            sorted_roots+=("$line")
        done < <(
            printf '%s\n' "${REX_PLAN_DELETE_ROOTS[@]}" |
            awk -F'\t' '{ depth=split($3,a,"/"); print depth "\t" $0 }' |
            sort -t$'\t' -k1,1rn -k3,3 |
            cut -f2-
        )
        REX_PLAN_DELETE_ROOTS=("${sorted_roots[@]}")
    fi
}
