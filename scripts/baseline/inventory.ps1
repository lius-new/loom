param([string]$Repository, [string]$OutputPath)
$ErrorActionPreference = 'Stop'
$sourceRoot = Join-Path $Repository 'src'
$sourceDirectories = @($sourceRoot)
$coreRoot = Join-Path $Repository 'crates/editor-core'
if (Test-Path -LiteralPath $coreRoot) { $sourceDirectories += $coreRoot }
$files = Get-ChildItem -LiteralPath $sourceDirectories -Recurse -Filter '*.rs'
$fileRecords = foreach ($file in $files) {
    $text = Get-Content -LiteralPath $file.FullName -Raw
    [pscustomobject]@{
        path = [IO.Path]::GetRelativePath($Repository, $file.FullName).Replace('\', '/')
        lines = @(Get-Content -LiteralPath $file.FullName).Count
        test_declarations = [regex]::Matches($text, '(?m)^\s*#\[test\]').Count
        sha256 = (Get-FileHash -LiteralPath $file.FullName -Algorithm SHA256).Hash
    }
}
$stateText = Get-Content -LiteralPath (Join-Path $sourceRoot 'app/state.rs') -Raw
$stateBody = [regex]::Match($stateText, '(?s)pub struct AppState \{(.*?)\r?\n\}').Groups[1].Value
$rules = @(
    @{ id = 'LEGACY-01'; files = 'model/*.rs'; pattern = 'lgui::|crate::theme|crate::fonts'; remove_in = 'R4' },
    @{ id = 'LEGACY-02'; files = 'model/workspace.rs'; pattern = 'crate::git|crate::search|crate::background_ui'; remove_in = 'R2 R4 R6' },
    @{ id = 'LEGACY-03'; files = 'model/*.rs'; pattern = 'std::fs::'; remove_in = 'R4 R5' },
    @{ id = 'LEGACY-04'; files = 'git/store.rs'; pattern = 'crate::model::diff_document'; remove_in = 'R6' },
    @{ id = 'LEGACY-05'; files = 'app/state.rs'; pattern = 'crate::ui::|ApplicationHandle|FileLoader'; remove_in = 'R1 R2 R5 R6 R7 R8' },
    @{ id = 'LEGACY-06'; files = 'vim/*.rs'; pattern = 'crate::input::keystroke'; remove_in = 'R3' }
)
$candidates = foreach ($rule in $rules) {
    foreach ($file in $files) {
        $relative = [IO.Path]::GetRelativePath($sourceRoot, $file.FullName).Replace('\', '/')
        if ($relative -notlike $rule.files) { continue }
        foreach ($match in Select-String -LiteralPath $file.FullName -Pattern $rule.pattern) {
            [pscustomobject]@{ id = $rule.id; path = 'src/' + $relative; line = $match.LineNumber; text = $match.Line.Trim(); remove_in = $rule.remove_in }
        }
    }
}
[ordered]@{
    schema = 1
    scope = 'Application and editor-core source identities; lexical candidates include cfg(test), not a Rust dependency parser.'
    app_state_public_fields = [regex]::Matches($stateBody, '(?m)^\s+pub \w+:').Count
    files = @($fileRecords)
    dependency_candidates = @($candidates)
} | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath $OutputPath -Encoding utf8
