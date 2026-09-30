param(
    [Parameter(Mandatory = $true)][string]$SourceRoot,
    [Parameter(Mandatory = $true)][string]$PackageRoot,
    [Parameter(Mandatory = $true)][string]$Version,
    [Parameter(Mandatory = $true)][string]$SourceUrl
)

$ErrorActionPreference = 'Stop'
$source = (Resolve-Path -LiteralPath $SourceRoot).Path
$runtime = Join-Path $PackageRoot 'runtime/git'
New-Item -ItemType Directory -Path $runtime -Force | Out-Null
Get-ChildItem -LiteralPath $source -Force |
    Copy-Item -Destination $runtime -Recurse -Force
$Version | Set-Content -LiteralPath (Join-Path $runtime 'VERSION') -Encoding ascii

$files = Get-ChildItem -LiteralPath $runtime -Recurse -File |
    Where-Object { $_.Name -ne 'MANIFEST.json' } |
    Sort-Object FullName |
    ForEach-Object {
        $relative = [IO.Path]::GetRelativePath($runtime, $_.FullName).Replace('\', '/')
        [ordered]@{
            path = $relative
            sha256 = (Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
        }
    }

$manifest = [ordered]@{
    version = $Version
    platform = [Environment]::OSVersion.Platform.ToString()
    architecture = [Runtime.InteropServices.RuntimeInformation]::OSArchitecture.ToString()
    source = $SourceUrl
    generated_at = [DateTimeOffset]::UtcNow.ToString('O')
    files = @($files)
}
$manifest | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath (Join-Path $runtime 'MANIFEST.json') -Encoding utf8
