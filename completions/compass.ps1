Register-ArgumentCompleter -Native -CommandName compass -ScriptBlock {
    param($wordToComplete, $commandAst, $cursorPosition)
    $tokens = $commandAst.CommandElements | ForEach-Object { $_.Extent.Text }
    $values = if ($tokens.Count -le 1) {
        @('update', 'extract', 'watch', 'serve', 'cluster-only', 'query', 'path', 'explain', 'affected', 'tree', 'export', 'benchmark', 'diagnose', 'merge-graphs', '--help', '--version')
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
