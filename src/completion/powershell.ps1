
    $previous = ''
    for ($i = 1; $i -lt $commandAst.CommandElements.Count; $i++) {
        $element = $commandAst.CommandElements[$i]
        if ($element.Extent.EndOffset -ge $cursorPosition) { break }
        $previous = $element.Extent.Text
        if ($previous -ceq '--' -or
            ($element -is [System.Management.Automation.Language.StringConstantExpressionAst] -and
             $element.Value -ceq '--')) { return }
    }

    $prefix = [string]$wordToComplete
    $leader = ''
    $comparison = [StringComparison]::Ordinal
    if ($prefix.StartsWith('--', [StringComparison]::Ordinal)) {
        $separator = $prefix.IndexOf('=')
        if ($separator -lt 0) {
            $candidates = $options.Keys
        } else {
            $option = $prefix.Substring(0, $separator)
            if ($options.Keys -cnotcontains $option) { return }
            $candidates = $options[$option]
            $leader = "$option="
            $prefix = $prefix.Substring($separator + 1)
        }
    } elseif ($options.Keys -ccontains $previous) {
        $candidates = $options[$previous]
    } else {
        return
    }

    foreach ($candidate in $candidates) {
        if ($candidate.StartsWith($prefix, $comparison)) {
            $text = $leader + $candidate
            if ($text -cnotmatch '\A[a-zA-Z0-9_=-]+\z') {
                $text = "'" + [System.Management.Automation.Language.CodeGeneration]::EscapeSingleQuotedStringContent($text) + "'"
            }
            [System.Management.Automation.CompletionResult]::new(
                $text, $candidate, [System.Management.Automation.CompletionResultType]::ParameterValue, $candidate
            )
        }
    }
}
