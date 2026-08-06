Register-ArgumentCompleter -Native -CommandName batch-git -ScriptBlock {
    param($wordToComplete, $commandAst, $cursorPosition)
    $commands = @(
        'add', 'branch', 'capabilities', 'cd', 'cf', 'checkout', 'clone', 'commit',
        'env', 'exec', 'fetch', 'find', 'forget', 'info', 'list', 'merge', 'pull',
        'push', 'restore', 'scan', 'schedule', 'schema', 'status', 'sync', 'unstage'
    )
    $options = @(
        '--jobs', '--verbose', '--output', '--request-id', '--non-interactive',
        '--timeout', '--plan', '--apply', '--expect-workspace-revision', '--help', '--version'
    )
    $tokens = $commandAst.CommandElements | ForEach-Object { $_.Extent.Text }
    $candidates = if ($tokens.Count -le 2) { $commands + $options } else { $options }
    foreach ($candidate in $candidates) {
        if ($candidate -like "$wordToComplete*") {
            [System.Management.Automation.CompletionResult]::new(
                $candidate, $candidate, 'ParameterValue', $candidate
            )
        }
    }
}
