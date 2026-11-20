#!/usr/bin/env bash
set -euo pipefail

printf 'Bash %s (%s)\n' "$BASH_VERSION" "$OSTYPE"
# Match the startup instructions: evaluate only successful generation.
script=$(remove-except --generate-completion bash)
eval "$script"
unset script

assert_candidates() {
    local previous=$1 current=$2 expected=$3
    COMP_WORDS=("$command_name")
    if [[ -n $previous ]]; then COMP_WORDS+=("$previous"); fi
    COMP_WORDS+=("$current")
    COMP_CWORD=$((${#COMP_WORDS[@]} - 1))
    "$function_name" "$command_name" "$current" "$previous"
    if [[ ${COMPREPLY[*]} != "$expected" ]]; then
        printf '%s completion after <%s> for <%s>: expected <%s>, got <%s>\n' \
            "$command_name" "$previous" "$current" "$expected" "${COMPREPLY[*]}" >&2
        exit 1
    fi
}

base_registration=$(complete -p remove-except)
for command_name in remove-except remove-except.exe; do
    registration=$(complete -p "$command_name")
    [[ ${registration% "$command_name"} == "${base_registration% remove-except}" ]]
    [[ $registration == *'-o bashdefault'* && $registration == *'-o default'* ]]
    function_name=${registration#* -F }
    function_name=${function_name%% *}
    declare -F "$function_name" >/dev/null

    assert_candidates '' '--gen' '--generate-completion'
    assert_candidates '' '-n' '-n'
    assert_candidates --generate-completion '' 'bash powershell'
    assert_candidates --format j 'json'
    assert_candidates --pager a 'auto always'
    assert_candidates --layout '' 'tree flat'
    assert_candidates --summary b 'both'
done
