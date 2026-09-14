$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$root = Split-Path -Parent $PSScriptRoot
Push-Location $root
try {
    $metadataJson = cargo metadata --locked --format-version 1 --no-deps | Out-String
    if ($LASTEXITCODE -ne 0) {
        throw "cargo metadata failed with exit code $LASTEXITCODE"
    }

    $metadata = $metadataJson | ConvertFrom-Json
    $package = @($metadata.packages | Where-Object { $_.name -eq "llmwire" })
    if ($package.Count -ne 1) {
        throw "expected exactly one llmwire package, found $($package.Count)"
    }

    $expectedDirect = @("bitflags", "serde", "serde_json", "thiserror") | Sort-Object
    $actualDirect = @(
        $package[0].dependencies |
            Where-Object { [string]::IsNullOrEmpty($_.kind) -or $_.kind -eq "normal" } |
            ForEach-Object { $_.name } |
            Sort-Object -Unique
    )

    if (($actualDirect -join "`n") -ne ($expectedDirect -join "`n")) {
        throw "core direct dependencies differ: expected [$($expectedDirect -join ', ')]; actual [$($actualDirect -join ', ')]"
    }

    $tree = cargo tree --locked -p llmwire --edges normal --prefix none --format "{p}" | Out-String
    if ($LASTEXITCODE -ne 0) {
        throw "cargo tree failed with exit code $LASTEXITCODE"
    }

    $forbidden = @("anyhow", "async-trait", "reqwest", "tokio")
    foreach ($name in $forbidden) {
        if ($tree -match "(?m)^$([regex]::Escape($name)) v") {
            throw "forbidden core dependency found: $name"
        }
    }

    Write-Host "core dependency policy: ok"
}
finally {
    Pop-Location
}
