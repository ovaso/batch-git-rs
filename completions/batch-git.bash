# shellcheck shell=bash disable=SC2207
# `compgen` intentionally returns one completion candidate per shell word for COMPREPLY.
_batch_git() {
    local current previous
    current="${COMP_WORDS[COMP_CWORD]}"
    previous="${COMP_WORDS[COMP_CWORD-1]}"
    local commands="add branch capabilities cd cf checkout clone commit env exec fetch find forget info list merge pull push restore scan schedule schema status sync unstage"
    local globals="--jobs --verbose --output --request-id --non-interactive --timeout --plan --apply --expect-workspace-revision --help --version"

    if [ "$COMP_CWORD" -eq 1 ]; then
        COMPREPLY=( $(compgen -W "$commands $globals" -- "$current") )
        return
    fi
    case "${COMP_WORDS[1]}" in
        env) COMPREPLY=( $(compgen -W "list" -- "$current") ) ;;
        schedule) COMPREPLY=( $(compgen -W "add doctor generate list plan register remove run status unregister update" -- "$current") ) ;;
        schema) COMPREPLY=( $(compgen -W "operation-result workspace" -- "$current") ) ;;
        *)
            case "$previous" in
                --output) COMPREPLY=( $(compgen -W "text json jsonl" -- "$current") ) ;;
                *) COMPREPLY=( $(compgen -W "$globals" -- "$current") ) ;;
            esac
            ;;
    esac
}
complete -F _batch_git batch-git
