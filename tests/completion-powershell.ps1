$ErrorActionPreference = 'Stop'
Write-Host "PowerShell $($PSVersionTable.PSVersion) ($($PSVersionTable.PSEdition))"

# Match the startup instructions, including joining native stdout lines in 5.1.
Get-Command remove-except -CommandType Application -ErrorAction Stop | Out-Null
$removeExceptCompletion = remove-except --generate-completion powershell
if ($LASTEXITCODE -ne 0) { throw 'Completion generation failed' }
. ([scriptblock]::Create(($removeExceptCompletion -join "`n")))
Remove-Variable removeExceptCompletion

function Assert-Candidates {
    param([string] $Line, [string[]] $Expected, [int] $Cursor = $Line.Length)
    $result = [System.Management.Automation.CommandCompletion]::CompleteInput($Line, $Cursor, $null)
    $actual = @($result.CompletionMatches | ForEach-Object { $_.CompletionText })
    if ((($actual | Sort-Object) -join "`n") -cne (($Expected | Sort-Object) -join "`n")) {
        throw "Completion of <$Line> at ${Cursor}: expected <$Expected>, got <$actual>"
    }
}

$fixture = Join-Path ([System.IO.Path]::GetTempPath()) "remove-except-completion-$([guid]::NewGuid())"
New-Item -ItemType Directory -Path $fixture | Out-Null
Push-Location -LiteralPath $fixture
try {
    New-Item -ItemType File -Path completion-fixture.txt | Out-Null
    Assert-Candidates 'remove-except --gen' '--generate-completion'
    Assert-Candidates 'remove-except.exe --gen' '--generate-completion'
    Assert-Candidates 'remove-except --help --ver' '--version'
    Assert-Candidates 'remove-except --generate-completion ' @('bash', 'powershell')
    Assert-Candidates 'remove-except --format j' 'json'
    Assert-Candidates 'remove-except --layout ' @('tree', 'flat')
    Assert-Candidates 'remove-except --summary=b' '--summary=both'
    Assert-Candidates 'remove-except --pager=' @('--pager=auto', '--pager=always', '--pager=never')
    Assert-Candidates 'remove-except --pager a' @('auto', 'always')
    Assert-Candidates 'remove-except --pager [a]' @()
    Assert-Candidates 'remove-except --pager nonsense' @()
    Assert-Candidates 'remove-except -- --gen' @()
    Assert-Candidates 'remove-except "--" --gen' @()
    Assert-Candidates 'remove-except --gen -- other' '--generate-completion' 'remove-except --gen'.Length
    $file = '.' + [System.IO.Path]::DirectorySeparatorChar + 'completion-fixture.txt'
    Assert-Candidates 'remove-except --root completion-fi' $file
    Assert-Candidates 'remove-except -- completion-fi' $file
} finally {
    Pop-Location
    Remove-Item -LiteralPath (Join-Path $fixture 'completion-fixture.txt'), $fixture
}
