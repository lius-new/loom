#requires -Version 7.0
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'skia-cache.ps1')
if (-not $IsWindows) { throw 'Run the Windows cache tests on Windows.' }
$repository = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../..'))
$fixture = Join-Path $repository ('target/skia-cache-tests/' + [guid]::NewGuid())
New-Item -ItemType Directory -Path $fixture -Force | Out-Null
$previousUrl = $env:SKIA_BINARIES_URL
$hadSkiaUrl = Test-Path Env:SKIA_BINARIES_URL
Copy-Item -LiteralPath (Join-Path $repository 'Cargo.lock') -Destination (Join-Path $fixture 'Cargo.lock')

function Expect-Failure([scriptblock]$Operation, [string]$Message) {
    $failure = $null
    try { & $Operation | Out-Null } catch { $failure = $_.Exception.Message }
    if (-not $failure -or $failure -notlike "*$Message*") {
        throw "Expected failure containing '$Message', got '$failure'."
    }
}

if ($null -ne (Resolve-LoomSkiaCache -Repository $fixture)) {
    throw 'An absent cache must allow the normal upstream build path.'
}
Write-Host 'PASS: an absent cache leaves normal dependency resolution available.'
Expect-Failure { Resolve-LoomSkiaCache -Repository $fixture -ArchivePath (Join-Path $fixture 'missing.tar.gz') } 'not found'
Write-Host 'PASS: an explicitly missing archive fails.'
$corrupt = Join-Path $fixture 'corrupt.tar.gz'
Set-Content -LiteralPath $corrupt -Value 'not a Skia archive' -Encoding utf8
Expect-Failure { Resolve-LoomSkiaCache -Repository $fixture -ArchivePath $corrupt } 'SHA-256 mismatch'
Write-Host 'PASS: a corrupt or different archive fails before use.'
$lockPath = Join-Path $fixture 'Cargo.lock'
$lock = Get-Content -LiteralPath $lockPath -Raw
$lock.Replace('version = "0.99.0"', 'version = "0.98.0"') | Set-Content -LiteralPath $lockPath -Encoding utf8
Expect-Failure { Resolve-LoomSkiaCache -Repository $fixture } 'does not match Cargo.lock'
Write-Host 'PASS: a changed locked Skia version requires a manifest update.'
if ($env:SKIA_BINARIES_URL -ne $previousUrl) { throw 'Cache resolution changed the caller environment.' }
Write-Host 'PASS: resolution preserves the caller environment.'

$cache = Resolve-LoomSkiaCache -Repository $repository
if ($cache) {
    $previousLocation = (Get-Location).Path
    try {
        $env:SKIA_BINARIES_URL = 'caller-owned-url'
        & (Join-Path $PSScriptRoot 'cargo.ps1') -SkiaArchive $cache.archive -CargoArguments @('--version') | Out-Null
        if ($LASTEXITCODE -ne 0) { throw 'The wrapper failed to invoke Cargo.' }
        if ($env:SKIA_BINARIES_URL -ne 'caller-owned-url' -or (Get-Location).Path -ne $previousLocation) {
            throw 'The wrapper failed to restore its caller environment or location.'
        }
        Write-Host 'PASS: explicit archive overrides are scoped to the Cargo invocation.'
        & (Join-Path $PSScriptRoot 'cargo.ps1') -SkiaArchive $cache.archive -CargoArguments @('--loom-preflight-invalid-option') *> (Join-Path $fixture 'cargo-failure.log')
        if ($LASTEXITCODE -eq 0) { throw 'The wrapper hid a Cargo failure.' }
        if ($env:SKIA_BINARIES_URL -ne 'caller-owned-url') { throw 'The wrapper leaked an override after failure.' }
        Write-Host 'PASS: Cargo failures remain nonzero and the override is restored.'
        Remove-Item Env:SKIA_BINARIES_URL
        & (Join-Path $PSScriptRoot 'cargo.ps1') -SkiaArchive $cache.archive -CargoArguments @('--version') | Out-Null
        if ($LASTEXITCODE -ne 0 -or (Test-Path Env:SKIA_BINARIES_URL)) {
            throw 'The wrapper created a caller environment variable that was absent.'
        }
        Write-Host 'PASS: an absent caller environment variable remains absent.'
        $env:SKIA_BINARIES_URL = ''
        & (Join-Path $PSScriptRoot 'cargo.ps1') -SkiaArchive $cache.archive -CargoArguments @('--version') | Out-Null
        if ($LASTEXITCODE -ne 0 -or -not (Test-Path Env:SKIA_BINARIES_URL) -or $env:SKIA_BINARIES_URL -ne '') {
            throw 'The wrapper failed to preserve an explicitly empty environment variable.'
        }
        Write-Host 'PASS: an explicitly empty caller environment variable is preserved.'
    } finally {
        if ($hadSkiaUrl) { $env:SKIA_BINARIES_URL = $previousUrl }
        else { Remove-Item Env:SKIA_BINARIES_URL -ErrorAction SilentlyContinue }
    }
}
exit 0
