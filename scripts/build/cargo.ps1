#requires -Version 7.0
[CmdletBinding(PositionalBinding = $false)]
param(
    [string]$SkiaArchive,
    [Parameter(Position = 0, ValueFromRemainingArguments)][string[]]$CargoArguments
)
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'skia-cache.ps1')
$repository = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../..'))
$previousUrl = [Environment]::GetEnvironmentVariable('SKIA_BINARIES_URL', 'Process')
$hadSkiaUrl = Test-Path Env:SKIA_BINARIES_URL
$previousLocation = Get-Location
$code = 1
try {
    Set-Location -LiteralPath $repository
    if ($SkiaArchive -or -not $previousUrl) {
        $cache = Resolve-LoomSkiaCache -Repository $repository -ArchivePath $SkiaArchive
        if ($cache) {
            $env:SKIA_BINARIES_URL = $cache.url
            Write-Host "Verified local Skia $($cache.version): $($cache.archive)"
        }
    }
    & cargo @CargoArguments
    $code = $LASTEXITCODE
} finally {
    if ($hadSkiaUrl) {
        $env:SKIA_BINARIES_URL = $previousUrl
    } else {
        Remove-Item Env:SKIA_BINARIES_URL -ErrorAction SilentlyContinue
    }
    Set-Location -LiteralPath $previousLocation.Path
}
exit $code
