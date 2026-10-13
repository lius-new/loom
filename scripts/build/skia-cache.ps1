#requires -Version 7.0
# Resolving a cache only reads files. Callers scope the returned URL to their process.
function Resolve-LoomSkiaCache {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)][string]$Repository,
        [string]$ArchivePath
    )

    if (-not $IsWindows) {
        if ($ArchivePath) { throw 'This Skia cache manifest supports Windows x64 only.' }
        return $null
    }
    $manifest = Get-Content -LiteralPath (Join-Path $PSScriptRoot 'skia-cache.json') -Raw | ConvertFrom-Json
    $lock = Get-Content -LiteralPath (Join-Path $Repository 'Cargo.lock') -Raw
    $version = [regex]::Match($lock, '(?m)^name = "skia-bindings"\r?\nversion = "([^"]+)"').Groups[1].Value
    if ($version -ne $manifest.version) {
        throw "Skia cache manifest version $($manifest.version) does not match Cargo.lock ($version). Update and verify the cache manifest first."
    }

    $candidates = [Collections.Generic.List[string]]::new()
    if ($ArchivePath) {
        $candidates.Add([IO.Path]::GetFullPath($ArchivePath))
    } else {
        $candidates.Add((Join-Path $Repository "target/dependency-cache/skia/$version/$($manifest.archive)"))
        foreach ($profile in @('debug', 'release')) {
            $build = Join-Path $Repository "target/$profile/build"
            if (Test-Path -LiteralPath $build) {
                foreach ($directory in Get-ChildItem -LiteralPath $build -Directory -Filter 'skia-bindings-*') {
                    $candidates.Add((Join-Path $directory.FullName "out/.cache/$($manifest.archive)"))
                }
            }
        }
    }
    foreach ($candidate in $candidates) {
        if (-not (Test-Path -LiteralPath $candidate -PathType Leaf)) { continue }
        $digest = (Get-FileHash -LiteralPath $candidate -Algorithm SHA256).Hash.ToLowerInvariant()
        if ($digest -ne $manifest.sha256) {
            throw "Skia archive SHA-256 mismatch: $candidate. Expected $($manifest.sha256), got $digest."
        }
        $absolute = (Resolve-Path -LiteralPath $candidate).Path
        return [pscustomobject]@{
            archive = $absolute
            version = $version
            key = $manifest.key
            sha256 = $digest
            source = $manifest.source
            url = 'file://' + ($absolute -replace '\\', '/')
        }
    }
    if ($ArchivePath) { throw "Skia archive not found: $ArchivePath" }
    return $null
}
