
$ErrorActionPreference = "Stop"

$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

$release = Join-Path $root "release"
$build = Join-Path $root "dist-sea"
$packageRoot = Join-Path $build "packages"

$versionFile = Join-Path $root "app-version.json"
$publicSource = Join-Path $root "public"
$ytDlpSource = Join-Path $root "vendor\yt-dlp.exe"
$ffmpegSource = Join-Path $root "vendor\ffmpeg\bin\ffmpeg.exe"

$cargoManifest = Join-Path $root "launcher-rust\Cargo.toml"
$rustLauncher = Join-Path $root "launcher-rust\target\release\CypherSR.exe"

$repository = "Cypherusuh/CypherSR"

$launcherName = "CypherSR.exe"
$coreName = "CypherSR-core.exe"
$versionJsonName = "version.json"

# ------------------------------------------------------------
# 1. Validate build inputs and load app version
# ------------------------------------------------------------

if (-not (Test-Path -LiteralPath $versionFile -PathType Leaf)) {
    throw "Missing app-version.json"
}

$versionConfig = Get-Content -LiteralPath $versionFile -Raw |
    ConvertFrom-Json

$appVersion = [string]$versionConfig.version

if ($appVersion -notmatch '^\d+\.\d+\.\d+$') {
    throw "app-version.json must contain a version like 1.0.0"
}

# Version-specific update asset name.
$updateAssetName = "update-only-$appVersion.zip"

# GitHub release tags are assumed to use the v prefix.
$updateUrl = "https://github.com/$repository/releases/download/v$appVersion/$updateAssetName"

foreach ($path in @(
    $publicSource,
    $ytDlpSource,
    $ffmpegSource,
    $cargoManifest
)) {
    if (-not (Test-Path -LiteralPath $path)) {
        throw "Required build input not found: $path"
    }
}

foreach ($commandName in @("node", "npx", "cargo")) {
    if (-not (Get-Command $commandName -ErrorAction SilentlyContinue)) {
        throw "$commandName is required to build CypherSR."
    }
}

New-Item -ItemType Directory -Force -Path `
    $release, $build, $packageRoot | Out-Null

# Preserve user data. Never clean or package release\data.
$dataDirectory = Join-Path $release "data"
New-Item -ItemType Directory -Force -Path $dataDirectory |
    Out-Null

Write-Host "CypherSR version: $appVersion"
Write-Host "Update asset:     $updateAssetName"

# ------------------------------------------------------------
# 2. Build the Node.js Single Executable Application
#    This builds CypherSR-core.exe only.
# ------------------------------------------------------------

function Build-SeaExecutable {
    param(
        [Parameter(Mandatory = $true)]
        [string]$EntryPoint,

        [Parameter(Mandatory = $true)]
        [string]$OutputName
    )

    $entryPath = Join-Path $root $EntryPoint

    if (-not (Test-Path -LiteralPath $entryPath -PathType Leaf)) {
        throw "Entry point not found: $entryPath"
    }

    $name = [System.IO.Path]::GetFileNameWithoutExtension($OutputName)
    $bundle = Join-Path $build "$name.cjs"
    $blob = Join-Path $build "$name.blob"
    $configPath = Join-Path $build "$name-sea-config.json"
    $outputExe = Join-Path $release $OutputName
    $temporaryExe = Join-Path $build "$name.building.exe"

    Remove-Item -LiteralPath @(
        $bundle,
        $blob,
        $configPath,
        $temporaryExe
    ) -Force -ErrorAction SilentlyContinue

    Write-Host ""
    Write-Host "Bundling $EntryPoint -> $OutputName"

    npx esbuild $entryPath `
        --bundle `
        --platform=node `
        --format=cjs `
        --target=node24 `
        --log-override:empty-import-meta=error `
        "--outfile=$bundle"

    if ($LASTEXITCODE -ne 0 -or
        -not (Test-Path -LiteralPath $bundle -PathType Leaf)) {
        throw "esbuild failed for $EntryPoint"
    }

    $seaConfig = @{
        main = $bundle
        output = $blob
        disableExperimentalSEAWarning = $true
        useCodeCache = $false
        useSnapshot = $false
    } | ConvertTo-Json

    [System.IO.File]::WriteAllText(
        $configPath,
        $seaConfig,
        [System.Text.UTF8Encoding]::new($false)
    )

    node --experimental-sea-config $configPath

    if ($LASTEXITCODE -ne 0 -or
        -not (Test-Path -LiteralPath $blob -PathType Leaf)) {
        throw "SEA blob preparation failed for $OutputName"
    }

    $nodeCommand = Get-Command node -ErrorAction Stop
    $nodeExe = $nodeCommand.Source

    Copy-Item -LiteralPath $nodeExe `
        -Destination $temporaryExe -Force

    npx postject $temporaryExe NODE_SEA_BLOB $blob `
        --sentinel-fuse NODE_SEA_FUSE_fce680ab2cc467b6e072b8b5df1996b2

    if ($LASTEXITCODE -ne 0) {
        Remove-Item -LiteralPath $temporaryExe `
            -Force -ErrorAction SilentlyContinue

        throw "SEA injection failed for $OutputName"
    }

    if (-not (Test-Path -LiteralPath $temporaryExe -PathType Leaf)) {
        throw "Build output was not created: $temporaryExe"
    }

    # Close the app first if Windows locks the destination executable.
    Move-Item -LiteralPath $temporaryExe `
        -Destination $outputExe -Force

    Write-Host "Created $outputExe"
}

Build-SeaExecutable `
    -EntryPoint "src/main.ts" `
    -OutputName $coreName

# ------------------------------------------------------------
# 3. Build the Rust launcher
#    The icon is embedded by launcher-rust/build.rs.
# ------------------------------------------------------------

Write-Host ""
Write-Host "Building Rust launcher..."

cargo build --release --manifest-path $cargoManifest

if ($LASTEXITCODE -ne 0) {
    throw "Rust launcher build failed."
}

if (-not (Test-Path -LiteralPath $rustLauncher -PathType Leaf)) {
    throw "Rust launcher executable not found: $rustLauncher"
}

$releaseLauncher = Join-Path $release $launcherName

Copy-Item -LiteralPath $rustLauncher `
    -Destination $releaseLauncher -Force

Write-Host "Created $releaseLauncher"

# ------------------------------------------------------------
# 4. Copy runtime assets into release
#    Preserve release\data.
# ------------------------------------------------------------

Write-Host ""
Write-Host "Copying application assets..."

$releasePublic = Join-Path $release "public"
$releaseVendor = Join-Path $release "vendor"

Remove-Item -LiteralPath @(
    $releasePublic,
    $releaseVendor
) -Recurse -Force -ErrorAction SilentlyContinue

Copy-Item -LiteralPath $publicSource `
    -Destination $releasePublic -Recurse -Force

$releaseYtDlp = Join-Path $releaseVendor "yt-dlp.exe"
$releaseFfmpegDir = Join-Path $releaseVendor "ffmpeg\bin"

New-Item -ItemType Directory -Force -Path `
    (Split-Path -Parent $releaseYtDlp),
    $releaseFfmpegDir | Out-Null

Copy-Item -LiteralPath $ytDlpSource `
    -Destination $releaseYtDlp -Force

Copy-Item -LiteralPath $ffmpegSource `
    -Destination (Join-Path $releaseFfmpegDir "ffmpeg.exe") -Force

# ------------------------------------------------------------
# 5. Write the installed version manifest
# ------------------------------------------------------------

$versionJson = @{
    version = $appVersion
} | ConvertTo-Json

[System.IO.File]::WriteAllText(
    (Join-Path $release $versionJsonName),
    $versionJson,
    [System.Text.UTF8Encoding]::new($false)
)

# ------------------------------------------------------------
# 6. Stage package contents
#
# Full ZIP:
#   CypherSR/CypherSR.exe
#   CypherSR/CypherSR-core.exe
#   CypherSR/version.json
#   CypherSR/public/
#   CypherSR/vendor/
#
# Update ZIP:
#   CypherSR-core.exe
#   version.json
#   public/
#   vendor/
#
# Neither ZIP contains data/.
# ------------------------------------------------------------

function Copy-UpdatePayload {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Destination
    )

    New-Item -ItemType Directory -Path $Destination -Force |
        Out-Null

    Copy-Item -LiteralPath (Join-Path $release $coreName) `
        -Destination $Destination -Force

    Copy-Item -LiteralPath (Join-Path $release $versionJsonName) `
        -Destination $Destination -Force

    Copy-Item -LiteralPath (Join-Path $release "public") `
        -Destination $Destination -Recurse -Force

    Copy-Item -LiteralPath (Join-Path $release "vendor") `
        -Destination $Destination -Recurse -Force
}

function Compress-StageZip {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Stage,

        [Parameter(Mandatory = $true)]
        [string]$Destination
    )

    Remove-Item -LiteralPath $Destination `
        -Force -ErrorAction SilentlyContinue

    # Archive the stage contents, not the stage directory itself.
    Compress-Archive `
        -Path (Join-Path $Stage "*") `
        -DestinationPath $Destination `
        -CompressionLevel Optimal

    if (-not (Test-Path -LiteralPath $Destination -PathType Leaf)) {
        throw "Failed to create ZIP: $Destination"
    }
}

$fullStage = Join-Path $packageRoot "full"
$updateStage = Join-Path $packageRoot "update"

foreach ($stage in @($fullStage, $updateStage)) {
    Remove-Item -LiteralPath $stage `
        -Recurse -Force -ErrorAction SilentlyContinue

    New-Item -ItemType Directory -Path $stage -Force |
        Out-Null
}

# Build the complete portable package inside a CypherSR folder.
$fullAppStage = Join-Path $fullStage "CypherSR"

New-Item -ItemType Directory -Path $fullAppStage -Force |
    Out-Null

Copy-UpdatePayload -Destination $fullAppStage

Copy-Item -LiteralPath (Join-Path $release $launcherName) `
    -Destination $fullAppStage -Force

# Build the update package without the launcher or user data.
Copy-UpdatePayload -Destination $updateStage

$fullZip = Join-Path $release "CypherSR-$appVersion-win-x64.zip"
$updateZip = Join-Path $release $updateAssetName

Compress-StageZip -Stage $fullStage -Destination $fullZip
Compress-StageZip -Stage $updateStage -Destination $updateZip

# ------------------------------------------------------------
# 7. Generate latest.json
#
# Publish latest.json as a separate asset on the GitHub release.
# The URL in this manifest points to the version-specific ZIP.
# ------------------------------------------------------------

$sha256 = (
    Get-FileHash -LiteralPath $updateZip -Algorithm SHA256
).Hash.ToLowerInvariant()

$manifest = @{
    version = $appVersion
    url = $updateUrl
    sha256 = $sha256
} | ConvertTo-Json

$manifestPath = Join-Path $release "latest.json"

[System.IO.File]::WriteAllText(
    $manifestPath,
    $manifest,
    [System.Text.UTF8Encoding]::new($false)
)

# ------------------------------------------------------------
# 8. Verify release artifacts and manifest
# ------------------------------------------------------------

$requiredReleaseFiles = @(
    (Join-Path $release $launcherName),
    (Join-Path $release $coreName),
    (Join-Path $release $versionJsonName),
    (Join-Path $release "public\player.html"),
    (Join-Path $release "vendor\yt-dlp.exe"),
    (Join-Path $release "vendor\ffmpeg\bin\ffmpeg.exe"),
    $fullZip,
    $updateZip,
    $manifestPath
)

foreach ($path in $requiredReleaseFiles) {
    if (-not (Test-Path -LiteralPath $path -PathType Leaf)) {
        throw "Release verification failed; missing file: $path"
    }
}

$writtenManifest = Get-Content -LiteralPath $manifestPath -Raw |
    ConvertFrom-Json

$expectedUpdateUrl = $updateUrl

if ($writtenManifest.url -ne $expectedUpdateUrl) {
    throw "Manifest URL does not match the expected version-specific update URL."
}

if ($writtenManifest.version -ne $appVersion) {
    throw "Manifest version does not match app-version.json."
}

if ($writtenManifest.sha256 -ne $sha256) {
    throw "Manifest SHA-256 verification failed."
}

if ($writtenManifest.sha256 -notmatch '^[a-f0-9]{64}$') {
    throw "Manifest SHA-256 is not a valid 64-character lowercase hexadecimal hash."
}

# ------------------------------------------------------------
# 9. Build summary
# ------------------------------------------------------------

Write-Host ""
Write-Host "============================================"
Write-Host "CypherSR build completed successfully"
Write-Host "============================================"
Write-Host "App version: $appVersion"
Write-Host ""
Write-Host "Launcher:    $releaseLauncher"
Write-Host "Core:        $(Join-Path $release $coreName)"
Write-Host "Full ZIP:    $fullZip"
Write-Host "Update ZIP:  $updateZip"
Write-Host "Manifest:    $manifestPath"
Write-Host ""
Write-Host "Update URL:  $updateUrl"
Write-Host "SHA-256:     $sha256"
Write-Host ""
Write-Host "The Rust launcher is built with Cargo."
Write-Host "The update ZIP excludes the launcher and user data."
Write-Host "latest.json is a separate GitHub release asset."
Write-Host "Existing release\data was preserved."
Write-Host "Neither ZIP contains the data folder."
Write-Host "============================================"
