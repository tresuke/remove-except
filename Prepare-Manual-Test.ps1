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

$instructions = @'
Manual test workspace

This folder is disposable. Re-run Prepare-Manual-Test.ps1 to restore the generated files.
The generator overwrites only its known fixture files and does not remove other files.

From this directory, preview the result:
  cargo run --manifest-path ..\Cargo.toml -- --dry-run keep.txt keep *.md

Then try the confirmation prompt (answer n to leave files unchanged):
  cargo run --manifest-path ..\Cargo.toml -- keep.txt keep *.md

To actually remove the listed entries without a prompt, use --force instead of --dry-run.
Run the generator again to restore the fixture afterwards.
'@
[System.IO.File]::WriteAllText((Join-Path $workspace 'MANUAL-TEST.txt'), $instructions, $utf8)

Write-Output "Manual test workspace ready: $workspace"
