[CmdletBinding(SupportsShouldProcess = $true, ConfirmImpact = 'High')]
# Usage: Remove-Except.ps1 -Patterns @('dist', '*.md') [-DryRun] [-Force] [-WhatIf]
param(
    [Parameter(Mandatory = $true)]
    [ValidateNotNullOrEmpty()]
    [string[]] $Patterns,

    [switch] $DryRun,

    [switch] $Force
)

. "$PSScriptRoot\Remove-Except.Common.ps1"

$plan = Get-RemoveExceptPlan -RootPath (Get-Location).ProviderPath -Patterns $Patterns

if ($plan.DirectMatches.Count -eq 0) {
    Write-Warning 'No items matched the keep patterns. Everything under the current directory would be removed.'
}

$plan.DeleteItems |
    Select-Object @{ Name = 'ItemType'; Expression = { if ($_.IsDirectory) { 'Directory' } else { 'File' } } }, RelativePath, FullName

if ($DryRun) {
    return
}

if ($Force -and -not $PSBoundParameters.ContainsKey('Confirm')) {
    $ConfirmPreference = 'None'
}

foreach ($item in $plan.DeleteRoots) {
    if ($PSCmdlet.ShouldProcess($item.FullName, 'Remove')) {
        Remove-Item -LiteralPath $item.FullName -Recurse -Force
    }
}

