$ErrorActionPreference = "Stop"

$repositoryRoot = Split-Path -Parent $PSScriptRoot
$steps = @(
    "build-backend.ps1",
    "build-desktop.ps1",
    "publish-package.ps1"
)

Push-Location $repositoryRoot

try {
    foreach ($step in $steps) {
        Write-Host ""
        Write-Host "============================================"
        Write-Host "Running $step"
        Write-Host "============================================"

        & (Join-Path $PSScriptRoot $step)

        if (-not $?) {
            throw "$step failed."
        }
    }
}
finally {
    Pop-Location
}

Write-Host ""
Write-Host "Release build and packaging completed successfully."
