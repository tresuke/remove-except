#!/usr/bin/env bash
set -euo pipefail

if (( $# > 1 )); then
    printf 'Usage: bash prepare-manual-test.sh [workspace-path]\n' >&2
    exit 1
fi

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
workspace="$(realpath -m -- "${1:-$script_dir/manual-test-workspace}")"
link_path="$workspace/keep-link"
link_target="$workspace/keep.txt"

if [[ -L "$link_path" ]]; then
    existing_target="$(readlink -- "$link_path")"
    if [[ "$existing_target" != /* ]]; then
        existing_target="$workspace/$existing_target"
    fi
    if [[ "$(realpath -m -- "$existing_target")" != "$link_target" ]]; then
        printf 'Cannot refresh the symbolic-link fixture: %s is not the expected link to %s.\n' "$link_path" "$link_target" >&2
        exit 1
    fi
elif [[ -e "$link_path" ]]; then
    printf 'Cannot refresh the symbolic-link fixture: %s is not a symbolic link.\n' "$link_path" >&2
    exit 1
fi

paths=(
    'keep.txt'
    'release-notes.md'
    'remove.txt'
    'keep/README.md'
    'keep/nested/important.txt'
    'delete-me/nested/temporary.tmp'
    'logs/app.log'
    '.hidden-example'
)
contents=(
    'Keep this root-level file.'
    'Keep this Markdown file.'
    'This file should be removed.'
    'This nested Markdown file is kept by the keep directory pattern.'
    'This file is kept by the keep directory pattern.'
    'This nested directory should be removed.'
    'This log should be removed.'
    'Hidden entries are included in the plan.'
)

for index in "${!paths[@]}"; do
    file_path="$workspace/${paths[$index]}"
    mkdir -p -- "$(dirname -- "$file_path")"
    printf '%s\n' "${contents[$index]}" > "$file_path"
done

if [[ -L "$link_path" ]]; then
    rm -- "$link_path"
fi
ln -s -- 'keep.txt' "$link_path"

cat > "$workspace/MANUAL-TEST.txt" <<'INSTRUCTIONS'
Manual test workspace

Re-run prepare-manual-test.sh to restore the generated files.
The generator overwrites only its known fixture files, refreshes the known keep-link to keep.txt, and does not remove other files.
keep-link is a symbolic link to keep.txt.

From this directory, preview the result:
  cargo run --manifest-path ../Cargo.toml -- --dry-run keep.txt keep '*.md'

Then try the confirmation prompt (answer n to leave files unchanged):
  cargo run --manifest-path ../Cargo.toml -- keep.txt keep '*.md'

To actually remove the listed entries without a prompt, use --force instead of --dry-run.
Run the generator again to restore the fixtures afterwards.
For a custom workspace path, use the absolute path to the repository's Cargo.toml instead of ../Cargo.toml.
INSTRUCTIONS

printf 'Manual test workspace ready: %s\n' "$workspace"