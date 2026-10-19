#requires -Version 7.0
[CmdletBinding(PositionalBinding = $false)]
param(
    [string]$SkiaArchive,
    [Parameter(Position = 0, ValueFromRemainingArguments)][string[]]$CargoArguments
)
$ErrorActionPreference = 'Stop'
$arguments = @()
if ($SkiaArchive) { $arguments += @('--skia-archive', [IO.Path]::GetFullPath($SkiaArchive)) }
& node (Join-Path $PSScriptRoot 'cargo.mjs') @arguments @CargoArguments
exit $LASTEXITCODE
