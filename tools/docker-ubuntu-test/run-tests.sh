#!/usr/bin/env bash
set -Eeuo pipefail

source_root=/workspace-input
workspace_root=$(mktemp -d "$HOME/remove-except-docker-test.XXXXXX")
trap 'rm -rf "$workspace_root"' EXIT

tar -C "$source_root" \
    --exclude='./.git' \
    --exclude='./target' \
    --exclude='./local-test-tools' \
    --exclude='./manual-test-workspace' \
    --exclude='./.github/copilot-instructions.md' \
    --exclude='./remove-except-decisions.md' \
    -cf - . | tar -C "$workspace_root" -xf -

for excluded_path in \
    .git \
    target \
    local-test-tools \
    manual-test-workspace \
    .github/copilot-instructions.md \
    remove-except-decisions.md; do
    if [[ -e "$workspace_root/$excluded_path" ]]; then
        printf 'Local or generated path was copied into the test workspace: %s\n' "$excluded_path" >&2
        exit 1
    fi
done

cd "$workspace_root"

run_checks() {
    local toolchain=$1
    local target_dir="$workspace_root/target/$toolchain"

    printf '\n=== Ubuntu 26.04 / Rust %s ===\n' "$toolchain"
    rustc +"$toolchain" --version
    cargo +"$toolchain" --version

    CARGO_TARGET_DIR="$target_dir" cargo +"$toolchain" fmt --check
    CARGO_TARGET_DIR="$target_dir" cargo +"$toolchain" test --locked
    CARGO_TARGET_DIR="$target_dir" cargo +"$toolchain" clippy --all-targets --locked -- -D warnings
}

run_checks 1.88.0
run_checks stable