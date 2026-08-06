set -l batch_git_commands add branch capabilities cd cf checkout clone commit env exec fetch find forget info list merge pull push restore scan schedule schema status sync unstage
complete -c batch-git -f
complete -c batch-git -n "not __fish_seen_subcommand_from $batch_git_commands" -a "$batch_git_commands"
complete -c batch-git -l jobs -r -d 'Maximum parallel repositories'
complete -c batch-git -l verbose -d 'Show successful child output'
complete -c batch-git -l output -r -a 'text json jsonl' -d 'Machine output format'
complete -c batch-git -l request-id -r -d 'Request identifier'
complete -c batch-git -l non-interactive -d 'Disable child prompts'
complete -c batch-git -l timeout -r -d 'Child process timeout'
complete -c batch-git -l plan -d 'Render a no-side-effect plan'
complete -c batch-git -l apply -d 'Apply against an expected workspace revision'
complete -c batch-git -l expect-workspace-revision -r -d 'Expected workspace revision'
complete -c batch-git -n '__fish_seen_subcommand_from env' -a list
complete -c batch-git -n '__fish_seen_subcommand_from schedule' -a 'add doctor generate list plan register remove run status unregister update'
complete -c batch-git -n '__fish_seen_subcommand_from schema' -a 'operation-result workspace'
