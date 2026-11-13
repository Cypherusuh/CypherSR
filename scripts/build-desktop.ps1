
$ErrorActionPreference = "Stop"

# CypherSR Desktop frontend build.
# Compiles the Rust/Slint GUI and creates a versioned ZIP release asset.
# The desktop version is read from desktop/Cargo.toml (independent from backend).
# The ZIP contains only CypherSR-Desktop.exe.
# Project root is detected relative to this script.

$FrontendRoot = Split-Path -Parent $PSScriptRoot
$DesktopRoot = Join-Path $FrontendRoot "desktop"
$CargoManifest = Join-Path $DesktopRoot "Cargo.toml"
$ReleaseDir = Join-Path $DesktopRoot "target\release"
$ReleaseRoot = Join-Path $FrontendRoot "release"

function Require-Path {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Path,

        [string]$Description = "Required path"
    )

    if (-not (Test-Path -LiteralPath $Path)) {
        throw "$Description not found: $Path"
    }
}

function Get-DesktopVersion {
    param(
        [Parameter(Mandatory = $true)]
        [string]$ManifestPath
    )

    $manifest = Get-Content -LiteralPath $ManifestPath -Raw
    $inPackage = $false
    $version = $null

    foreach ($line in ($manifest -split "`r?`n")) {
        if ($line -match '^\s*\[(.+)\]\s*$') {
            $inPackage = ($Matches[1] -eq 'package')
            continue
        }

        if ($inPackage -and $line -match '^\s*version\s*=\s*"([^"]+)"\s*$') {
            $version = $Matches[1]
            break
        }
    }

    if ($null -eq $version -or $version -notmatch '^\d+\.\d+\.\d+$') {
        throw "desktop/Cargo.toml must contain a [package] version in the format 1.0.0"
    }

    return $version
}

Require-Path -Path $CargoManifest -Description "Desktop Cargo.toml"

if (-not (Get-Command "cargo" -ErrorAction SilentlyContinue)) {
    throw "Cargo was not found. Install Rust and reopen the terminal."
}

if (-not (Get-Command "Compress-Archive" -ErrorAction SilentlyContinue)) {
    throw "Compress-Archive is unavailable. Use a supported Windows PowerShell version."
}

$DesktopVersion = Get-DesktopVersion -ManifestPath $CargoManifest

$BuiltExePath = Join-Path $ReleaseDir "CypherSR-Desktop.exe"
$VersionedReleaseDir = Join-Path $ReleaseRoot "v$DesktopVersion"
$DesktopBuildDir = Join-Path $VersionedReleaseDir "Desktop Build"
$PublishingZipDir = Join-Path $VersionedReleaseDir "Publishing zip"
$VersioningDir = Join-Path $VersionedReleaseDir "versioning"
$DesktopBuildExePath = Join-Path $DesktopBuildDir "CypherSR-Desktop.exe"
$VersionedZipName = "CypherSR-Desktop-GUI-v$DesktopVersion-win-x64.zip"
$VersionedZipPath = Join-Path $PublishingZipDir $VersionedZipName

Write-Host ""
Write-Host "============================================"
Write-Host "Building CypherSR Desktop"
Write-Host "============================================"
Write-Host "Project:         $DesktopRoot"
Write-Host "Desktop version: $DesktopVersion"
Write-Host "Build output:    $DesktopBuildExePath"
Write-Host "Release asset:   $VersionedZipPath"
Write-Host ""

Push-Location $DesktopRoot

try {
    cargo build --release --manifest-path $CargoManifest

    if ($LASTEXITCODE -ne 0) {
        throw "Desktop build failed with exit code $LASTEXITCODE."
    }
}
finally {
    Pop-Location
}

Require-Path -Path $ReleaseDir -Description "Cargo release output directory"
Require-Path -Path $BuiltExePath -Description "Built Desktop executable"

$builtFile = Get-Item -LiteralPath $BuiltExePath

if ($builtFile.Length -le 0) {
    throw "The built Desktop executable is empty: $BuiltExePath"
}

# Create the versioned build and publishing directories without touching other
# release assets.
New-Item -ItemType Directory -Path @(
    $DesktopBuildDir,
    $PublishingZipDir,
    $VersioningDir
) -Force | Out-Null

Copy-Item -LiteralPath $BuiltExePath -Destination $DesktopBuildExePath -Force

# Use a temporary staging directory so the ZIP contains only the intended EXE.
$StageDir = Join-Path $PublishingZipDir ".desktop-zip-staging"
$StagedExePath = Join-Path $StageDir "CypherSR-Desktop.exe"
$TempZipPath = Join-Path $PublishingZipDir ".tmp-$VersionedZipName"

try {
    if (Test-Path -LiteralPath $StageDir) {
        Remove-Item -LiteralPath $StageDir -Recurse -Force
    }

    if (Test-Path -LiteralPath $TempZipPath) {
        Remove-Item -LiteralPath $TempZipPath -Force
    }

    New-Item -ItemType Directory -Path $StageDir -Force | Out-Null

    Copy-Item `
        -LiteralPath $BuiltExePath `
        -Destination $StagedExePath `
        -Force

    $stagedFile = Get-Item -LiteralPath $StagedExePath

    if ($stagedFile.Length -ne $builtFile.Length) {
        throw "The staged executable size does not match the build output."
    }

    # Compress only the staged executable.
    Compress-Archive `
        -LiteralPath $StagedExePath `
        -DestinationPath $TempZipPath `
        -CompressionLevel Optimal `
        -Force

    Require-Path -Path $TempZipPath -Description "Temporary Desktop GUI ZIP"

    $zipFile = Get-Item -LiteralPath $TempZipPath

    if ($zipFile.Length -le 0) {
        throw "The generated Desktop GUI ZIP is empty."
    }

    # Replace the final ZIP only after compression succeeds.
    Move-Item `
        -LiteralPath $TempZipPath `
        -Destination $VersionedZipPath `
        -Force
}
finally {
    # Always remove temporary build staging files.
    if (Test-Path -LiteralPath $StageDir) {
        Remove-Item -LiteralPath $StageDir -Recurse -Force
    }

    if (Test-Path -LiteralPath $TempZipPath) {
        Remove-Item -LiteralPath $TempZipPath -Force
    }
}

Require-Path -Path $VersionedZipPath -Description "Versioned Desktop GUI ZIP"

$desktopVersionJson = @{
    version = $DesktopVersion
} | ConvertTo-Json

New-Item -ItemType Directory -Path $VersioningDir -Force | Out-Null
[System.IO.File]::WriteAllText(
    (Join-Path $VersioningDir "desktop.json"),
    $desktopVersionJson,
    [System.Text.UTF8Encoding]::new($false)
)
# Unified versioning manifest for future consumers (kept alongside legacy folder).
$versioningManifestPath = Join-Path $VersionedReleaseDir "versioning.json"
$vComponents = [ordered]@{}
if (Test-Path -LiteralPath $versioningManifestPath) {
    try {
        $existing = Get-Content -LiteralPath $versioningManifestPath -Raw | ConvertFrom-Json
        if ($existing.components) {
            foreach ($p in $existing.components.PSObject.Properties) { $vComponents[$p.Name] = $p.Value }
        }
    } catch { }
}
$vComponents["desktop"] = [ordered]@{ version = $DesktopVersion }
[System.IO.File]::WriteAllText(
    $versioningManifestPath,
    ([ordered]@{ schemaVersion = 1; components = $vComponents } | ConvertTo-Json -Depth 20),
    [System.Text.UTF8Encoding]::new($false)
)

$finalZip = Get-Item -LiteralPath $VersionedZipPath
$compressionRatio = if ($builtFile.Length -gt 0) {
    [math]::Round((1 - ($finalZip.Length / $builtFile.Length)) * 100, 1)
}
else {
    0
}

Write-Host ""
Write-Host "============================================"
Write-Host "Desktop build completed successfully"
Write-Host "============================================"
Write-Host "Desktop version: $DesktopVersion"
Write-Host "Build output:    $DesktopBuildExePath"
Write-Host "Release asset:   $VersionedZipPath"
Write-Host "EXE size:        $([math]::Round($builtFile.Length / 1MB, 2)) MB"
Write-Host "ZIP size:        $([math]::Round($finalZip.Length / 1MB, 2)) MB"
Write-Host "Size reduction:  $compressionRatio%"
Write-Host ""
