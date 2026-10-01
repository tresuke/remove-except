function Normalize-RemoveExceptPath {
    param(
        [Parameter(Mandatory = $true)]
        [string] $Path,

        [switch] $TrimTrailingSeparator
    )

    $normalized = $Path.Replace('/', '\')

    if ($TrimTrailingSeparator) {
        if ($normalized -match '^[A-Za-z]:\\$' -or $normalized -eq '\\') {
            return $normalized
        }

        $normalized = $normalized.TrimEnd('\\')
    }

    return $normalized
}

function Test-RemoveExceptHasWildcard {
    param(
        [Parameter(Mandatory = $true)]
        [string] $Pattern
    )

    return $Pattern.IndexOfAny([char[]] '*?[') -ge 0
}

function Test-RemoveExceptParentTraversal {
    param(
        [Parameter(Mandatory = $true)]
        [string] $Pattern
    )

    return $Pattern -match '(^|[\\/])\.\.($|[\\/])'
}

function Test-RemoveExceptIsWithinRoot {
    param(
        [Parameter(Mandatory = $true)]
        [string] $RootPath,

        [Parameter(Mandatory = $true)]
        [string] $CandidatePath
    )

    $normalizedRoot = Normalize-RemoveExceptPath -Path $RootPath -TrimTrailingSeparator
    $normalizedCandidate = Normalize-RemoveExceptPath -Path $CandidatePath -TrimTrailingSeparator

    if ($normalizedCandidate -eq $normalizedRoot) {
        return $true
    }

    $relativePath = [System.IO.Path]::GetRelativePath($normalizedRoot, $normalizedCandidate)
    return -not (
        [System.IO.Path]::IsPathRooted($relativePath) -or
        $relativePath -eq '..' -or
        $relativePath.StartsWith('..\', [System.StringComparison]::OrdinalIgnoreCase)
    )
}

function Join-RemoveExceptAbsolutePattern {
    param(
        [Parameter(Mandatory = $true)]
        [string] $RootPath,

        [Parameter(Mandatory = $true)]
        [string] $RelativePattern
    )

    if ([string]::IsNullOrEmpty($RelativePattern)) {
        return $RootPath
    }

    return Normalize-RemoveExceptPath -Path ("$RootPath\$RelativePattern")
}

function Get-RemoveExceptDirectoryPrefix {
    param(
        [Parameter(Mandatory = $true)]
        [string] $Pattern
    )

    if ([string]::IsNullOrWhiteSpace($Pattern)) {
        return ''
    }

    $normalized = Normalize-RemoveExceptPath -Path $Pattern

    if ($normalized.EndsWith('\*')) {
        return Normalize-RemoveExceptPath -Path $normalized.Substring(0, $normalized.Length - 2) -TrimTrailingSeparator
    }

    if ($normalized.EndsWith('\')) {
        return Normalize-RemoveExceptPath -Path $normalized -TrimTrailingSeparator
    }

    if (-not (Test-RemoveExceptHasWildcard -Pattern $normalized)) {
        return Normalize-RemoveExceptPath -Path $normalized -TrimTrailingSeparator
    }

    return $null
}

function New-RemoveExceptPatternSpec {
    param(
        [Parameter(Mandatory = $true)]
        [string] $Pattern,

        [Parameter(Mandatory = $true)]
        [string] $RootPath
    )

    $rawPattern = $Pattern.Trim()
    if ([string]::IsNullOrWhiteSpace($rawPattern)) {
        throw 'Pattern must not be empty.'
    }

    if (Test-RemoveExceptParentTraversal -Pattern $rawPattern) {
        throw "Parent directory traversal is not allowed in pattern: $Pattern"
    }

    $normalizedRoot = Normalize-RemoveExceptPath -Path $RootPath -TrimTrailingSeparator
    $isAbsolute = [System.IO.Path]::IsPathRooted($rawPattern)

    if ($isAbsolute) {
        $absolutePattern = Normalize-RemoveExceptPath -Path $rawPattern
        $relativePattern = $null

        if (-not (Test-RemoveExceptHasWildcard -Pattern $absolutePattern)) {
            $fullPath = Normalize-RemoveExceptPath -Path ([System.IO.Path]::GetFullPath($absolutePattern)) -TrimTrailingSeparator
            if (-not (Test-RemoveExceptIsWithinRoot -RootPath $normalizedRoot -CandidatePath $fullPath)) {
                throw "Absolute pattern is outside the current root: $Pattern"
            }

            $absolutePattern = $fullPath

            if ($fullPath -eq $normalizedRoot) {
                $relativePattern = ''
            }
            else {
                $relativePattern = Normalize-RemoveExceptPath -Path ([System.IO.Path]::GetRelativePath($normalizedRoot, $fullPath))
            }
        }
        elseif ($absolutePattern.StartsWith($normalizedRoot, [System.StringComparison]::OrdinalIgnoreCase)) {
            $relativeCandidate = $absolutePattern.Substring($normalizedRoot.Length).TrimStart('\\')
            $relativePattern = Normalize-RemoveExceptPath -Path $relativeCandidate
        }
    }
    else {
        $relativePattern = Normalize-RemoveExceptPath -Path ($rawPattern -replace '^(\.[\\/])+', '')
        $relativePattern = $relativePattern.TrimStart('\\')
        $absolutePattern = Join-RemoveExceptAbsolutePattern -RootPath $normalizedRoot -RelativePattern $relativePattern
    }

    [pscustomobject]@{
        Original        = $Pattern
        RelativePattern = $relativePattern
        AbsolutePattern = $absolutePattern
        RelativePrefix  = if ($null -ne $relativePattern) { Get-RemoveExceptDirectoryPrefix -Pattern $relativePattern } else { $null }
        AbsolutePrefix  = Get-RemoveExceptDirectoryPrefix -Pattern $absolutePattern
    }
}

function Get-RemoveExceptMatchDetail {
    param(
        [Parameter(Mandatory = $true)]
        [string] $RelativePath,

        [Parameter(Mandatory = $true)]
        [string] $AbsolutePath,

        [Parameter(Mandatory = $true)]
        [psobject] $PatternSpec
    )

    $relativeLike = $false
    $absoluteLike = $false
    $relativePrefix = $false
    $absolutePrefix = $false

    if ($null -ne $PatternSpec.RelativePattern) {
        $relativeLike = $RelativePath -like $PatternSpec.RelativePattern
    }

    if ($null -ne $PatternSpec.AbsolutePattern) {
        $absoluteLike = $AbsolutePath -like $PatternSpec.AbsolutePattern
    }

    if ($null -ne $PatternSpec.RelativePrefix) {
        $relativePrefix = [string]::IsNullOrEmpty($PatternSpec.RelativePrefix) -or
            $RelativePath -eq $PatternSpec.RelativePrefix -or
            $RelativePath.StartsWith("$($PatternSpec.RelativePrefix)\", [System.StringComparison]::OrdinalIgnoreCase)
    }

    if ($null -ne $PatternSpec.AbsolutePrefix) {
        $absolutePrefix = $AbsolutePath -eq $PatternSpec.AbsolutePrefix -or
            $AbsolutePath.StartsWith("$($PatternSpec.AbsolutePrefix)\", [System.StringComparison]::OrdinalIgnoreCase)
    }

    [pscustomobject]@{
        Pattern        = $PatternSpec.Original
        RelativeLike   = $relativeLike
        AbsoluteLike   = $absoluteLike
        RelativePrefix = $relativePrefix
        AbsolutePrefix = $absolutePrefix
        Matched        = $relativeLike -or $absoluteLike -or $relativePrefix -or $absolutePrefix
    }
}

function Get-RemoveExceptPlan {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory = $true)]
        [string] $RootPath,

        [Parameter(Mandatory = $true)]
        [string[]] $Patterns
    )

    if (-not $Patterns -or $Patterns.Count -eq 0) {
        throw 'At least one pattern is required.'
    }

    $normalizedRoot = Normalize-RemoveExceptPath -Path ([System.IO.Path]::GetFullPath($RootPath)) -TrimTrailingSeparator
    $patternSpecs = $Patterns | ForEach-Object { New-RemoveExceptPatternSpec -Pattern $_ -RootPath $normalizedRoot }
    $itemByPath = @{}
    $directMatches = New-Object System.Collections.Generic.List[object]

    Get-ChildItem -LiteralPath $normalizedRoot -Recurse -Force | ForEach-Object {
        $item = $_
        $absolutePath = Normalize-RemoveExceptPath -Path $item.FullName -TrimTrailingSeparator
        $relativePath = Normalize-RemoveExceptPath -Path ([System.IO.Path]::GetRelativePath($normalizedRoot, $absolutePath))

        $itemInfo = [pscustomobject]@{
            FullName     = $absolutePath
            RelativePath = $relativePath
            Name         = $item.Name
            IsDirectory  = $item.PSIsContainer
            Item         = $item
        }

        $itemByPath[$absolutePath] = $itemInfo

        $matchDetails = foreach ($patternSpec in $patternSpecs) {
            Get-RemoveExceptMatchDetail -RelativePath $relativePath -AbsolutePath $absolutePath -PatternSpec $patternSpec
        }

        if ($matchDetails.Where({ $_.Matched }, 'First').Count -gt 0) {
            $directMatches.Add([pscustomobject]@{
                Item         = $itemInfo
                MatchDetails = $matchDetails
            })
        }
    }

    $keepPaths = New-Object 'System.Collections.Generic.HashSet[string]' ([System.StringComparer]::OrdinalIgnoreCase)

    foreach ($match in $directMatches) {
        $currentPath = $match.Item.FullName

        while ($currentPath -and $keepPaths.Add($currentPath)) {
            if ($currentPath -eq $normalizedRoot) {
                break
            }

            $parentPath = [System.IO.Path]::GetDirectoryName($currentPath)
            if ([string]::IsNullOrEmpty($parentPath)) {
                break
            }

            $currentPath = Normalize-RemoveExceptPath -Path $parentPath -TrimTrailingSeparator
            if (-not (Test-RemoveExceptIsWithinRoot -RootPath $normalizedRoot -CandidatePath $currentPath)) {
                break
            }
        }
    }

    $deleteItems = foreach ($itemInfo in $itemByPath.Values) {
        if (-not $keepPaths.Contains($itemInfo.FullName)) {
            $itemInfo
        }
    }

    $deleteItemLookup = @{}
    foreach ($itemInfo in $deleteItems) {
        $deleteItemLookup[$itemInfo.FullName] = $itemInfo
    }

    $deleteRoots = foreach ($itemInfo in $deleteItems) {
        $parentPath = [System.IO.Path]::GetDirectoryName($itemInfo.FullName)
        $skipItem = $false

        while (-not [string]::IsNullOrEmpty($parentPath)) {
            $normalizedParent = Normalize-RemoveExceptPath -Path $parentPath -TrimTrailingSeparator
            if ($deleteItemLookup.ContainsKey($normalizedParent) -and $deleteItemLookup[$normalizedParent].IsDirectory) {
                $skipItem = $true
                break
            }

            if ($normalizedParent -eq $normalizedRoot) {
                break
            }

            $parentPath = [System.IO.Path]::GetDirectoryName($normalizedParent)
        }

        if (-not $skipItem) {
            $itemInfo
        }
    }

    [pscustomobject]@{
        RootPath      = $normalizedRoot
        PatternSpecs  = $patternSpecs
        DirectMatches = $directMatches
        KeepPaths     = $keepPaths
        DeleteItems   = $deleteItems | Sort-Object RelativePath
        DeleteRoots   = $deleteRoots | Sort-Object @{ Expression = { $_.FullName.Split('\\').Count } ; Descending = $true }, RelativePath
    }
}