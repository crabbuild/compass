Register-ArgumentCompleter -Native -CommandName trail -ScriptBlock {
    param($wordToComplete, $commandAst, $cursorPosition)
    $tokens = $commandAst.CommandElements | ForEach-Object { $_.Extent.Text }
    $values = if ($tokens.Count -le 1) {
        @('graph', '--help', '--version')
    } elseif ($tokens.Count -le 2) {
        @('update', 'extract', 'watch', 'cluster-only', 'query', 'path', 'explain', 'affected', 'tree', 'export', 'benchmark', 'diagnose', 'merge-graphs')
    } elseif ($tokens -contains 'export') {
        @('html', 'callflow-html', 'obsidian', 'wiki', 'svg', 'graphml')
    } elseif ($tokens -contains 'diagnose') {
        @('multigraph')
    } else {
        @('--help', '--graph', '--out', '--output', '--force', '--no-cluster', '--no-viz', '--exclude', '--resolution', '--exclude-hubs')
    }
    $values | Where-Object { $_ -like "$wordToComplete*" } | ForEach-Object {
        [System.Management.Automation.CompletionResult]::new($_, $_, 'ParameterValue', $_)
    }
}
