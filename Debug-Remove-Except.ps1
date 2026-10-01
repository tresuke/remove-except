param(
    [Parameter(Mandatory = $true)]
    [string[]] $Patterns
)

. "$PSScriptRoot\Remove-Except.Common.ps1"

$root = (Get-Location).ProviderPath
$plan = Get-RemoveExceptPlan -RootPath $root -Patterns $Patterns

Write-Host "root: $($plan.RootPath)"
Write-Host 'patterns:'
$plan.PatternSpecs | ForEach-Object {
    Write-Host "  original='$($_.Original)' rel='$($_.RelativePattern)' abs='$($_.AbsolutePattern)' relPrefix='$($_.RelativePrefix)' absPrefix='$($_.AbsolutePrefix)'"
}

Get-ChildItem -LiteralPath $plan.RootPath -Recurse -Force |
    ForEach-Object {
        $full = Normalize-RemoveExceptPath -Path $_.FullName -TrimTrailingSeparator
        $rel = Normalize-RemoveExceptPath -Path ([System.IO.Path]::GetRelativePath($plan.RootPath, $full))
        $keep = $plan.KeepPaths.Contains($full)

        Write-Host "item rel='$rel' full='$full' keep=$keep"

        foreach ($patternSpec in $plan.PatternSpecs) {
            $detail = Get-RemoveExceptMatchDetail -RelativePath $rel -AbsolutePath $full -PatternSpec $patternSpec
            Write-Host "  pattern='$($detail.Pattern)' relLike=$($detail.RelativeLike) absLike=$($detail.AbsoluteLike) relPrefix=$($detail.RelativePrefix) absPrefix=$($detail.AbsolutePrefix) matched=$($detail.Matched)"
        }
    }

