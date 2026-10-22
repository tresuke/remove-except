[CmdletBinding()]
param(
    [string]$DockerPath,
    [string]$DockerSourcePath
)

$ErrorActionPreference = 'Stop'

if ([string]::IsNullOrWhiteSpace($DockerPath)) {
    $dockerCommand = Get-Command docker -CommandType Application -ErrorAction Stop
    $dockerExecutable = $dockerCommand.Source
}
else {
    if (-not (Test-Path -LiteralPath $DockerPath -PathType Leaf)) {
        throw "Docker executable not found: $DockerPath"
    }

    $dockerExecutable = (Resolve-Path -LiteralPath $DockerPath).Path
}

$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$drive = $repoRoot.Substring(0, 1).ToLowerInvariant()
$relativePath = $repoRoot.Substring(2).Replace('\', '/')
$dockerSource = if ([string]::IsNullOrWhiteSpace($DockerSourcePath)) {
    "/mnt/$drive$relativePath"
}
else {
    $DockerSourcePath
}

if (-not $dockerSource.StartsWith('/')) {
    throw 'DockerSourcePath must be an absolute Linux path visible to the Docker Engine.'
}

$dockerContext = Join-Path $PSScriptRoot 'docker-ubuntu-test'
$imageTag = 'remove-except-ubuntu-test:local'

& $dockerExecutable build --tag $imageTag $dockerContext
if ($LASTEXITCODE -ne 0) {
    exit $LASTEXITCODE
}

$runArguments = @(
    'run'
    '--rm'
    '--mount'
    "type=bind,source=$dockerSource,target=/workspace-input,readonly"
    '--mount'
    'type=volume,source=remove-except-cargo-registry,target=/usr/local/cargo/registry'
    '--mount'
    'type=volume,source=remove-except-cargo-target,target=/cargo-target-cache'
    $imageTag
)

& $dockerExecutable @runArguments
if ($LASTEXITCODE -ne 0) {
    exit $LASTEXITCODE
}