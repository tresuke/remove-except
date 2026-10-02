param(
    [string] $Path = (Join-Path $PSScriptRoot 'manual-test-workspace')
)

$workspace = [System.IO.Path]::GetFullPath($Path)
$utf8 = New-Object System.Text.UTF8Encoding($false)

$fixtures = [ordered]@{
    'keep.txt'                         = 'Keep this root-level file.'
    'release-notes.md'                 = 'Keep this Markdown file.'
    'remove.txt'                       = 'This file should be removed.'
    'keep\README.md'                   = 'This nested Markdown file is also kept by *.md.'
    'keep\nested\important.txt'       = 'This file is kept by the keep directory pattern.'
    'delete-me\nested\temporary.tmp'  = 'This nested directory should be removed.'
    'logs\app.log'                    = 'This log should be removed.'
    '.hidden-example'                  = 'Hidden entries are included in the plan.'
}

foreach ($relativePath in $fixtures.Keys) {
    $filePath = Join-Path $workspace $relativePath
    $parent = Split-Path -Parent $filePath
    New-Item -ItemType Directory -Path $parent -Force | Out-Null
    [System.IO.File]::WriteAllText($filePath, $fixtures[$relativePath] + [Environment]::NewLine, $utf8)
}

 $linkPath = Join-Path $workspace 'keep-link'
 $linkTarget = Join-Path $workspace 'keep.txt'
 $existingLink = Get-Item -LiteralPath $linkPath -Force -ErrorAction SilentlyContinue
 if ($null -ne $existingLink) {
    if (
        $existingLink.LinkType -ne 'SymbolicLink' -or
        [System.IO.Path]::GetFullPath([string] $existingLink.Target) -ine $linkTarget
    ) {
        throw "Cannot refresh the symbolic-link fixture because '$linkPath' is not the expected link to '$linkTarget'."
    }

    Remove-Item -LiteralPath $linkPath -Force -ErrorAction Stop
 }

 try {
    New-Item -ItemType SymbolicLink -Path $linkPath -Target $linkTarget -ErrorAction Stop | Out-Null
 } catch {
    throw "Failed to create the symbolic-link fixture '$linkPath'. On Windows, enable Developer Mode or run PowerShell elevated. $($_.Exception.Message)"
 }

$instructions = @'
Manual test workspace

This folder is disposable. Re-run Prepare-Manual-Test.ps1 to restore the generated files.
The generator overwrites only its known fixture files, refreshes the known keep-link to keep.txt, and does not remove other files.
keep-link is a symbolic link to keep.txt. On Windows, creating it may require Developer Mode or an elevated PowerShell session.

From this directory, preview the result:
  cargo run --manifest-path ..\Cargo.toml -- --dry-run keep.txt keep *.md

Then try the confirmation prompt (answer n to leave files unchanged):
  cargo run --manifest-path ..\Cargo.toml -- keep.txt keep *.md

To actually remove the listed entries without a prompt, use --force instead of --dry-run.
Run the generator again to restore the fixture afterwards.
'@
[System.IO.File]::WriteAllText((Join-Path $workspace 'MANUAL-TEST.txt'), $instructions, $utf8)

Write-Output "Manual test workspace ready: $workspace"
