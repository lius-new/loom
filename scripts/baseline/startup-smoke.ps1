#requires -Version 7.0
[CmdletBinding()]
param(
    [string]$Executable,
    [Parameter(Mandatory)][string]$OutputDirectory
)
$ErrorActionPreference = 'Stop'
if (-not $IsWindows) { throw 'This startup smoke is for Windows.' }
$repository = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../..'))
if (-not $Executable) { $Executable = Join-Path $repository 'target/release/Loom.exe' }
$Executable = (Resolve-Path -LiteralPath $Executable).Path
$OutputDirectory = [IO.Path]::GetFullPath($OutputDirectory)
$resultPath = Join-Path $OutputDirectory 'startup-smoke.json'
if (Test-Path -LiteralPath $resultPath) { throw 'Choose a fresh startup smoke output directory.' }
$fixture = Join-Path $OutputDirectory 'startup-fixture'
$appdata = Join-Path $fixture 'appdata'
$localdata = Join-Path $fixture 'localappdata'
$workspace = Join-Path $fixture 'workspace'
New-Item -ItemType Directory -Path $appdata, $localdata, $workspace -Force | Out-Null
Set-Content -LiteralPath (Join-Path $workspace '示例.txt') -Value 'Loom startup smoke: 中文 + emoji 🙂' -Encoding utf8
$start = [Diagnostics.ProcessStartInfo]::new($Executable)
$start.UseShellExecute = $false
$start.WindowStyle = [Diagnostics.ProcessWindowStyle]::Hidden
$start.WorkingDirectory = $workspace
$start.ArgumentList.Add($workspace)
$start.Environment['APPDATA'] = $appdata
$start.Environment['LOCALAPPDATA'] = $localdata
$result = [ordered]@{
    exe = $Executable
    sha256 = (Get-FileHash -LiteralPath $Executable -Algorithm SHA256).Hash
    child_appdata = $appdata
    child_localappdata = $localdata
    window_style = 'Hidden'
    started_at = (Get-Date -Format o)
    result = 'failed'
    forced_cleanup = $false
}
$process = $null
try {
    $process = [Diagnostics.Process]::Start($start)
    $result.pid = $process.Id
    Start-Sleep -Seconds 4
    $process.Refresh()
    $result.alive_after_4_seconds = -not $process.HasExited
    $result.root_effect_configuration_files = @(Get-ChildItem -LiteralPath $appdata -Recurse -File | ForEach-Object Name)
    if ($process.HasExited) { throw "Loom exited during startup, code $($process.ExitCode)." }
    $result.close_requested = $process.CloseMainWindow()
    if (-not $result.close_requested -or -not $process.WaitForExit(10000)) {
        throw 'The isolated Loom process did not close normally.'
    }
    $result.exit_code = $process.ExitCode
    $result.clean_exit = $process.ExitCode -eq 0
    $session = Join-Path $appdata 'Loom/workspace.json'
    if (-not (Test-Path -LiteralPath $session)) { throw 'Loom did not save an isolated workspace session.' }
    $saved = Get-Content -LiteralPath $session -Raw | ConvertFrom-Json
    $savedFolders = @($saved.open_folders)
    $savedPath = if ($savedFolders.Count -eq 1) { [string]$savedFolders[0] } else { '' }
    # Rust canonicalization preserves the Win32 verbatim prefix for local paths.
    if ($savedPath.StartsWith('\\?\UNC\')) {
        $savedPath = '\\' + $savedPath.Substring(8)
    } elseif ($savedPath.StartsWith('\\?\')) {
        $savedPath = $savedPath.Substring(4)
    }
    if ($savedFolders.Count -ne 1 -or $savedPath -ne $workspace) {
        throw 'The saved session does not identify the synthetic workspace.'
    }
    $result.saved_synthetic_workspace = $true
    if (-not $result.clean_exit) { throw "Loom exited with code $($process.ExitCode)." }
    $result.result = 'passed'
} catch {
    $result.error = $_.Exception.Message
    throw
} finally {
    if ($process -and -not $process.HasExited) {
        # Only the process created above is eligible for forced test cleanup.
        $process.Kill()
        $process.WaitForExit()
        $result.forced_cleanup = $true
    }
    $result.finished_at = (Get-Date -Format o)
    $result | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath $resultPath -Encoding utf8
    if ($process) { $process.Dispose() }
}
Write-Host "Startup smoke passed: $resultPath"
