$ErrorActionPreference = 'Stop'

# Cargo/Tauri cache generated permission paths using the project location.
# Clean relocated caches before either development or release builds.
Push-Location (Join-Path $PSScriptRoot '..')
try {
    $projectRoot = (Get-Location).Path
    $targetDir = 'src-tauri/target'
    $markerPath = Join-Path $targetDir '.project-location'
    $previousRoot = if (Test-Path -LiteralPath $markerPath) {
        (Get-Content -LiteralPath $markerPath -Raw).Trim()
    } else {
        ''
    }

    if ((Test-Path -LiteralPath $targetDir) -and $previousRoot -ne $projectRoot) {
        Write-Host '[build] Project location changed or cache location unknown; cleaning Cargo cache...'
        & cargo clean --manifest-path src-tauri/Cargo.toml --target-dir $targetDir
        if ($LASTEXITCODE -ne 0) {
            throw 'Cargo cache cleanup failed.'
        }
    }

    New-Item -ItemType Directory -Path $targetDir -Force | Out-Null
    Set-Content -LiteralPath $markerPath -Value $projectRoot -Encoding UTF8
} finally {
    Pop-Location
}
