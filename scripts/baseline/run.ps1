#requires -Version 7.0
[CmdletBinding()]
param(
    [string]$OutputDirectory,
    [ValidateRange(1, 20)][int]$Samples = 5,
    [ValidateSet('All', 'Legacy', 'Additional', 'None')][string]$PerformanceSet = 'All',
    [string]$SkiaArchive,
    [switch]$Offline,
    [switch]$SkipChecks
)

$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $false
$repoRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../..'))
if (-not $OutputDirectory) {
    $OutputDirectory = Join-Path $repoRoot ('target/baseline/' + (Get-Date -Format 'yyyyMMdd-HHmmss'))
}
$OutputDirectory = [IO.Path]::GetFullPath($OutputDirectory)
$logDirectory = Join-Path $OutputDirectory 'logs'
if ((Test-Path -LiteralPath $logDirectory) -and
    @(Get-ChildItem -LiteralPath $logDirectory -Filter '*.log' -File).Count -gt 0) {
    throw 'Choose a new output directory so logs from different source versions are not mixed.'
}
New-Item -ItemType Directory -Force -Path $logDirectory | Out-Null
$cargoBase = @('--locked')
if ($Offline) { $cargoBase += '--offline' }
$checks = [Collections.Generic.List[object]]::new()
$previousLocation = Get-Location
$previousSkiaUrl = [Environment]::GetEnvironmentVariable('SKIA_BINARIES_URL', 'Process')
$hadSkiaUrl = Test-Path Env:SKIA_BINARIES_URL
$previousPath = $env:PATH
$previousCmake = [Environment]::GetEnvironmentVariable('CMAKE', 'Process')
$hadCmake = Test-Path Env:CMAKE
$previousForceDownload = [Environment]::GetEnvironmentVariable('FORCE_SKIA_BINARIES_DOWNLOAD', 'Process')
$hadForceDownload = Test-Path Env:FORCE_SKIA_BINARIES_DOWNLOAD
$previousForceBuild = [Environment]::GetEnvironmentVariable('FORCE_SKIA_BUILD', 'Process')
$hadForceBuild = Test-Path Env:FORCE_SKIA_BUILD
$previousSourceDir = [Environment]::GetEnvironmentVariable('SKIA_SOURCE_DIR', 'Process')
$hadSourceDir = Test-Path Env:SKIA_SOURCE_DIR
$previousLibrarySearch = [Environment]::GetEnvironmentVariable('SKIA_LIBRARY_SEARCH_PATH', 'Process')
$hadLibrarySearch = Test-Path Env:SKIA_LIBRARY_SEARCH_PATH

function Invoke-CargoCheck {
    param([string]$Name, [string[]]$Arguments)
    $started = Get-Date
    $logPath = Join-Path $logDirectory ($Name + '.log')
    & cargo @Arguments *> $logPath
    $code = $LASTEXITCODE
    $checks.Add([pscustomobject]@{
        name = $Name
        command = 'cargo ' + ($Arguments -join ' ')
        exit_code = $code
        elapsed_seconds = ((Get-Date) - $started).TotalSeconds
        log = 'logs/' + $Name + '.log'
    })
    Write-Host "$Name exit=$code"
    return $code
}

function Invoke-ArchitectureCheck {
    param([string]$Name, [string[]]$Arguments)
    $started = Get-Date
    $logPath = Join-Path $logDirectory ($Name + '.log')
    & node @Arguments *> $logPath
    $code = $LASTEXITCODE
    $checks.Add([pscustomobject]@{
        name = $Name
        command = 'node ' + ($Arguments -join ' ')
        exit_code = $code
        elapsed_seconds = ((Get-Date) - $started).TotalSeconds
        log = 'logs/' + $Name + '.log'
    })
    Write-Host "$Name exit=$code"
}

try {
    Set-Location -LiteralPath $repoRoot
    $dependencyArgs = @((Join-Path $PSScriptRoot '../dependencies/ensure.mjs'), '--json')
    if ($Offline) { $dependencyArgs += '--offline' }
    if ($SkiaArchive) { $dependencyArgs += @('--skia-archive', [IO.Path]::GetFullPath($SkiaArchive)) }
    $prepared = & node @dependencyArgs
    if ($LASTEXITCODE -ne 0) { throw 'Build dependency preparation failed.' }
    $dependencies = $prepared | ConvertFrom-Json
    $env:PATH = $dependencies.env.PATH
    $env:CMAKE = $dependencies.env.CMAKE
    $env:SKIA_BINARIES_URL = $dependencies.env.SKIA_BINARIES_URL
    Remove-Item Env:FORCE_SKIA_BINARIES_DOWNLOAD -ErrorAction SilentlyContinue
    Remove-Item Env:FORCE_SKIA_BUILD -ErrorAction SilentlyContinue
    Remove-Item Env:SKIA_SOURCE_DIR -ErrorAction SilentlyContinue
    Remove-Item Env:SKIA_LIBRARY_SEARCH_PATH -ErrorAction SilentlyContinue
    $skiaCache = $dependencies.skia
    $environment = [ordered]@{
        captured_at = (Get-Date -Format o)
        head = (git rev-parse HEAD)
        timezone = [TimeZoneInfo]::Local.Id
        rustc = (rustc -Vv | Out-String).Trim()
        cargo = (cargo -V)
        rustfmt = (rustfmt --version)
        clippy = (cargo clippy -V)
        git = (git --version)
        node = (node --version)
        os = [Environment]::OSVersion.VersionString
        processor = $env:PROCESSOR_IDENTIFIER
        logical_processors = [Environment]::ProcessorCount
        rustup_available = [bool](Get-Command rustup -ErrorAction SilentlyContinue)
        lock_sha256 = (Get-FileHash -LiteralPath Cargo.lock -Algorithm SHA256).Hash
        lock_lf_sha256 = [Convert]::ToHexString([Security.Cryptography.SHA256]::HashData(
            [Text.Encoding]::UTF8.GetBytes([IO.File]::ReadAllText((Join-Path $repoRoot 'Cargo.lock')).Replace("`r`n", "`n"))))
        manifest_sha256 = (Get-FileHash -LiteralPath Cargo.toml -Algorithm SHA256).Hash
        toolchain_sha256 = (Get-FileHash -LiteralPath rust-toolchain.toml -Algorithm SHA256).Hash
        lgui_source = ((Select-String -LiteralPath Cargo.lock -Pattern 'source = "git\+https://github.com/lius-new/lgui').Line | Select-Object -Unique)
        inherited_git_autocrlf = (git config --show-origin --get core.autocrlf)
        perf_samples = $Samples
        performance_set = $PerformanceSet
        warmup_samples_per_filter = 1
        skia_cache = $skiaCache
        skia_binaries_url = $env:SKIA_BINARIES_URL
    }
    try {
        $environment.hardware = [ordered]@{
            os = Get-CimInstance Win32_OperatingSystem | Select-Object Caption, Version, BuildNumber, OSArchitecture
            cpu = Get-CimInstance Win32_Processor | Select-Object Name, NumberOfCores, NumberOfLogicalProcessors
            physical_memory_bytes = (Get-CimInstance Win32_ComputerSystem).TotalPhysicalMemory
        }
    } catch {
        $environment.hardware = @{ unavailable = $_.Exception.Message }
    }
    $environment | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath (Join-Path $OutputDirectory 'environment.json') -Encoding utf8
    git status --porcelain | Set-Content -LiteralPath (Join-Path $OutputDirectory 'working-tree.txt') -Encoding utf8
    & (Join-Path $PSScriptRoot 'inventory.ps1') -Repository $repoRoot -OutputPath (Join-Path $OutputDirectory 'inventory.json')

    if (-not $SkipChecks) {
        Invoke-ArchitectureCheck 'architecture-self-test' @('--test', 'scripts/architecture/check.test.mjs')
        Invoke-ArchitectureCheck 'architecture-gate' @('scripts/architecture/check.mjs')
        Invoke-ArchitectureCheck 'baseline-report-self-test' @('--test', 'scripts/baseline/test-results.test.mjs')
        $null = Invoke-CargoCheck 'format' @('fmt', '--all', '--', '--check')
        $null = Invoke-CargoCheck 'clippy' (@('clippy') + $cargoBase + @('--workspace', '--all-targets', '--',
            '-D', 'warnings', '-A', 'clippy::redundant_closure_call',
            '-A', 'clippy::collapsible_if', '-A', 'clippy::too_many_arguments',
            '-A', 'clippy::let_and_return', '-A', 'clippy::manual_div_ceil'))
        $null = Invoke-CargoCheck 'tests-native' (@('test') + $cargoBase + @('--workspace', '--', '--nocapture'))
        $null = Invoke-CargoCheck 'tests-release' (@('test', '--release') + $cargoBase + @('--workspace', '--', '--nocapture'))
        $null = Invoke-CargoCheck 'release-app-build' (@('build', '--release') + $cargoBase + @('--bin', 'Loom'))
    }

    $previousNoSystem = [Environment]::GetEnvironmentVariable('GIT_CONFIG_NOSYSTEM', 'Process')
    $previousGlobal = [Environment]::GetEnvironmentVariable('GIT_CONFIG_GLOBAL', 'Process')
    $hadNoSystem = Test-Path Env:GIT_CONFIG_NOSYSTEM
    $hadGlobal = Test-Path Env:GIT_CONFIG_GLOBAL
    $controlledConfig = Join-Path $OutputDirectory 'controlled-git.config'
    # This synthetic file is scoped to child processes; no user config is changed.
    @'
[core]
autocrlf = false
eol = lf
'@ | Set-Content -LiteralPath $controlledConfig -Encoding utf8
    try {
        $env:GIT_CONFIG_NOSYSTEM = '1'
        $env:GIT_CONFIG_GLOBAL = $controlledConfig
        if (-not $SkipChecks) {
            $null = Invoke-CargoCheck 'tests-isolated' (@('test') + $cargoBase + @('--workspace', '--', '--nocapture'))
            if ($IsWindows) {
                $smokes = [ordered]@{
                    'native-terminal-smoke' = 'supported_shells_round_trip_through_real_conpty_sessions'
                    'native-launcher-smoke' = 'launcher_backed_shell_is_idle_then_detects_and_clears_programs'
                    'native-bash-replacement-smoke' = 'exited_shell_launchers_do_not_hide_unknown_applications'
                    'native-program-name-smoke' = 'running_application_is_detected_and_cleared_in_each_real_shell'
                }
                foreach ($smoke in $smokes.GetEnumerator()) {
                    $null = Invoke-CargoCheck $smoke.Key (@('test') + $cargoBase + @(
                        ('app::terminal_session_tests::tests::' + $smoke.Value),
                        '--', '--exact', '--ignored', '--nocapture', '--test-threads=1'))
                }
            }
        }
        $filters = [ordered]@{}
        if ($PerformanceSet -in @('All', 'Legacy')) {
            $filters['legacy-editor'] = 'app::editor_baseline_tests::perf_baseline'
            $filters['ui-interaction'] = 'app::editor::tests::large_file_interaction_baseline'
            $filters['tab-pointer'] = 'ui::tabs::tests::pointer_updates_with_many_tabs'
        }
        if ($PerformanceSet -in @('All', 'Additional')) {
            $filters['file'] = 'app::phase0_baseline::phase0_perf_file_preparation'
            $filters['search'] = 'app::phase0_baseline::phase0_perf_search_scan_and_cancel'
            $filters['terminal'] = 'app::phase0_baseline::phase0_perf_terminal_parser'
            $filters['terminal-production'] = 'app::terminal_performance::terminal_production_benchmark'
        }
        foreach ($item in $filters.GetEnumerator()) {
            for ($sample = 0; $sample -le $Samples; $sample++) {
                $code = Invoke-CargoCheck ("perf-" + $item.Key + "-" + $sample) (
                    @('test', '--release') + $cargoBase + @($item.Value,
                        '--', '--exact', '--ignored', '--nocapture', '--test-threads=1'))
                if ($code -ne 0) { break }
            }
        }
    } finally {
        if ($hadNoSystem) { $env:GIT_CONFIG_NOSYSTEM = $previousNoSystem }
        else { Remove-Item Env:GIT_CONFIG_NOSYSTEM -ErrorAction SilentlyContinue }
        if ($hadGlobal) { $env:GIT_CONFIG_GLOBAL = $previousGlobal }
        else { Remove-Item Env:GIT_CONFIG_GLOBAL -ErrorAction SilentlyContinue }
    }
    $checks | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath (Join-Path $OutputDirectory 'checks.json') -Encoding utf8
    & node (Join-Path $PSScriptRoot 'report.mjs') $OutputDirectory
    if ($LASTEXITCODE -ne 0) { throw 'Baseline report generation failed.' }
    if (($checks | Where-Object exit_code -NE 0).Count -gt 0) { exit 1 }
} finally {
    $env:PATH = $previousPath
    if ($hadCmake) { $env:CMAKE = $previousCmake }
    else { Remove-Item Env:CMAKE -ErrorAction SilentlyContinue }
    if ($hadForceDownload) { $env:FORCE_SKIA_BINARIES_DOWNLOAD = $previousForceDownload }
    else { Remove-Item Env:FORCE_SKIA_BINARIES_DOWNLOAD -ErrorAction SilentlyContinue }
    if ($hadForceBuild) { $env:FORCE_SKIA_BUILD = $previousForceBuild }
    else { Remove-Item Env:FORCE_SKIA_BUILD -ErrorAction SilentlyContinue }
    if ($hadSourceDir) { $env:SKIA_SOURCE_DIR = $previousSourceDir }
    else { Remove-Item Env:SKIA_SOURCE_DIR -ErrorAction SilentlyContinue }
    if ($hadLibrarySearch) { $env:SKIA_LIBRARY_SEARCH_PATH = $previousLibrarySearch }
    else { Remove-Item Env:SKIA_LIBRARY_SEARCH_PATH -ErrorAction SilentlyContinue }
    if ($hadSkiaUrl) { $env:SKIA_BINARIES_URL = $previousSkiaUrl }
    else { Remove-Item Env:SKIA_BINARIES_URL -ErrorAction SilentlyContinue }
    Set-Location -LiteralPath $previousLocation.Path
}
