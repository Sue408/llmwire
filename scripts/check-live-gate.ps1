$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$root = Split-Path -Parent $PSScriptRoot
Push-Location $root
try {
    $liveNames = @(
        Get-ChildItem -LiteralPath tests -Filter "live_*.rs" -File |
            ForEach-Object {
                Select-String -LiteralPath $_.FullName -Pattern '^fn\s+(live_[A-Za-z0-9_]+)' -AllMatches |
                    ForEach-Object { $_.Matches | ForEach-Object { $_.Groups[1].Value } }
            } |
            Sort-Object -Unique
    )

    if ($liveNames.Count -eq 0) {
        throw "no live_* test functions found"
    }

    $ignoredList = cargo test --locked -p llmwire -- --ignored --list | Out-String
    if ($LASTEXITCODE -ne 0) {
        throw "cargo test --ignored --list failed with exit code $LASTEXITCODE"
    }

    foreach ($name in $liveNames) {
        if ($ignoredList -notmatch "(?m)^\s*$([regex]::Escape($name)): test\s*$") {
            throw "live test is not ignored: $name"
        }
    }

    $trackedEnv = git ls-files -- .env
    if ($LASTEXITCODE -ne 0) {
        throw "git ls-files failed with exit code $LASTEXITCODE"
    }
    if (-not [string]::IsNullOrWhiteSpace($trackedEnv)) {
        throw ".env is tracked by git"
    }

    Write-Host "live opt-in gate: ok ($($liveNames.Count) ignored tests)"
}
finally {
    Pop-Location
}
