Register-ArgumentCompleter -Native -CommandName compass -ScriptBlock {
    param($wordToComplete, $commandAst, $cursorPosition)
    $tokens = $commandAst.CommandElements | ForEach-Object { $_.Extent.Text }
    $values = if ($tokens.Count -le 1) {
        @('init', 'update', 'extract', 'watch', 'serve', 'cluster-only', 'query', 'path', 'explain', 'affected', 'tree', 'export', 'benchmark', 'diagnose', 'merge-graphs', 'history', '--help', '--version')
    } elseif ($tokens -contains 'init') {
        @('--include', '--exclude', '--yes', '--force', '--help')
    } elseif (($tokens -contains 'history') -and $tokens.Count -le 2) {
        @('enable', 'disable', 'status', 'build', 'rebuild', 'list', 'show', 'prefer', 'export', 'gc')
    } elseif (($tokens -contains 'history') -and ($tokens -contains 'build')) {
        @('--all', '--first-parent', '--profile-from', '--format', '--code-only', '--backend', '--model', '--mode', '--cargo', '--dedup-llm', '--token-budget', '--resolution', '--exclude-hubs', '--no-gitignore', '--exclude')
    } elseif ($tokens -contains 'export') {
        @('html', 'callflow-html', 'obsidian', 'wiki', 'svg', 'graphml')
    } elseif ($tokens -contains 'diagnose') {
        @('multigraph')
    } elseif ($tokens -contains 'serve') {
        @('--help', '--graph', '--transport', '--host', '--port', '--api-key', '--path', '--json-response', '--stateless', '--session-timeout')
    } else {
        @('--help', '--graph', '--out', '--output', '--force', '--no-cluster', '--no-viz', '--exclude', '--resolution', '--exclude-hubs')
    }
    $values | Where-Object { $_ -like "$wordToComplete*" } | ForEach-Object {
        [System.Management.Automation.CompletionResult]::new($_, $_, 'ParameterValue', $_)
    }
}
