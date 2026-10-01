param(
    [Parameter(Mandatory = $true)]
    [string[]] $Patterns
)

. "$PSScriptRoot\Remove-Except.Common.ps1"

$plan = Get-RemoveExceptPlan -RootPath (Get-Location).ProviderPath -Patterns $Patterns

$plan.DeleteItems |
    Select-Object @{ Name = 'ItemType'; Expression = { if ($_.IsDirectory) { 'Directory' } else { 'File' } } }, RelativePath, FullName

