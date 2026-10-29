
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
$vendorVersionJsonName = "vendor-version.json"

# ------------------------------------------------------------
# 1. Validate build inputs and load versions
# ------------------------------------------------------------

if (-not (Test-Path -LiteralPath $versionFile -PathType Leaf)) {
    throw "Missing app-version.json"
}

try {
    $versionConfig = Get-Content -LiteralPath $versionFile -Raw |
        ConvertFrom-Json
}
catch {
    throw "Failed to parse app-version.json: $($_.Exception.Message)"
}

$appVersion = [string]$versionConfig.version
$vendorVersion = [string]$versionConfig.vendorVersion

if ($appVersion -notmatch '^\d+\.\d+\.\d+$') {
    throw "app-version.json must contain version in the format 1.0.0"
}

if ($vendorVersion -notmatch '^\d+\.\d+\.\d+$') {
    throw "app-version.json must contain vendorVersion in the format 1.0.0"
}

$updateAssetName = "update-only-$appVersion.zip"
$vendorAssetName = "vendor-$vendorVersion-win-x64.zip"
$fullAssetName = "CypherSR-$appVersion-win-x64.zip"

# Both assets are published under this app release tag.
# This script creates the vendor ZIP on every build, even when
# vendorVersion has not changed. Upload it with the other assets.
$releaseTag = "v$appVersion"
$releaseBaseUrl = "https://github.com/$repository/releases/download/$releaseTag"

$updateUrl = "$releaseBaseUrl/$updateAssetName"
$vendorUrl = "$releaseBaseUrl/$vendorAssetName"

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

if (-not (Test-Path -LiteralPath (Join-Path $publicSource "player.html") -PathType Leaf)) {
    throw "Required overlay not found: $publicSource\player.html"
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

Write-Host "CypherSR app version: $appVersion"
Write-Host "Vendor version:       $vendorVersion"
Write-Host "App update asset:     $updateAssetName"
Write-Host "Vendor asset:         $vendorAssetName"

# ------------------------------------------------------------
# 2. Build the Node.js Single Executable Application
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

    Move-Item -LiteralPath $temporaryExe `
        -Destination $outputExe -Force

    Write-Host "Created $outputExe"
}

Build-SeaExecutable `
    -EntryPoint "src/main.ts" `
    -OutputName $coreName

# ------------------------------------------------------------
# 3. Build the Rust launcher
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
# 4. Copy application and vendor assets into release
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

# Record the installed vendor version inside the vendor directory.
$vendorVersionJson = @{
    version = $vendorVersion
} | ConvertTo-Json

[System.IO.File]::WriteAllText(
    (Join-Path $releaseVendor $vendorVersionJsonName),
    $vendorVersionJson,
    [System.Text.UTF8Encoding]::new($false)
)

# ------------------------------------------------------------
# 5. Write the installed app version manifest
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
# App update ZIP:
#   CypherSR-core.exe
#   version.json
#   public/
#
# Vendor ZIP:
#   vendor/
#     yt-dlp.exe
#     ffmpeg/bin/ffmpeg.exe
#     vendor-version.json
#
# No ZIP contains data/.
# ------------------------------------------------------------

function Copy-AppUpdatePayload {
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

    Compress-Archive `
        -Path (Join-Path $Stage "*") `
        -DestinationPath $Destination `
        -CompressionLevel Optimal

    if (-not (Test-Path -LiteralPath $Destination -PathType Leaf)) {
        throw "Failed to create ZIP: $Destination"
    }
}

$fullStage = Join-Path $packageRoot "full"
$appUpdateStage = Join-Path $packageRoot "app-update"
$vendorStage = Join-Path $packageRoot "vendor-update"

foreach ($stage in @($fullStage, $appUpdateStage, $vendorStage)) {
    Remove-Item -LiteralPath $stage `
        -Recurse -Force -ErrorAction SilentlyContinue

    New-Item -ItemType Directory -Path $stage -Force |
        Out-Null
}

# Full portable package is wrapped in a CypherSR directory.
$fullAppStage = Join-Path $fullStage "CypherSR"

New-Item -ItemType Directory -Path $fullAppStage -Force |
    Out-Null

Copy-AppUpdatePayload -Destination $fullAppStage

Copy-Item -LiteralPath (Join-Path $release $launcherName) `
    -Destination $fullAppStage -Force

Copy-Item -LiteralPath (Join-Path $release "vendor") `
    -Destination $fullAppStage -Recurse -Force

# App-only update: deliberately excludes vendor and launcher.
Copy-AppUpdatePayload -Destination $appUpdateStage

# Vendor update: archive contains a vendor/ directory at its root.
Copy-Item -LiteralPath (Join-Path $release "vendor") `
    -Destination $vendorStage -Recurse -Force

$fullZip = Join-Path $release $fullAssetName
$updateZip = Join-Path $release $updateAssetName
$vendorZip = Join-Path $release $vendorAssetName

Compress-StageZip -Stage $fullStage -Destination $fullZip
Compress-StageZip -Stage $appUpdateStage -Destination $updateZip
Compress-StageZip -Stage $vendorStage -Destination $vendorZip

# ------------------------------------------------------------
# 7. Generate latest.json with independent app/vendor versions
# ------------------------------------------------------------

$appSha256 = (
    Get-FileHash -LiteralPath $updateZip -Algorithm SHA256
).Hash.ToLowerInvariant()

$vendorSha256 = (
    Get-FileHash -LiteralPath $vendorZip -Algorithm SHA256
).Hash.ToLowerInvariant()

$manifest = @{
    version = $appVersion
    url = $updateUrl
    sha256 = $appSha256
    vendor = $vendorVersion
    vendorUrl = $vendorUrl
    vendorSha256 = $vendorSha256
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
    (Join-Path $release "vendor\$vendorVersionJsonName"),
    $fullZip,
    $updateZip,
    $vendorZip,
    $manifestPath
)

foreach ($path in $requiredReleaseFiles) {
    if (-not (Test-Path -LiteralPath $path -PathType Leaf)) {
        throw "Release verification failed; missing file: $path"
    }
}

$writtenManifest = Get-Content -LiteralPath $manifestPath -Raw |
    ConvertFrom-Json

if ($writtenManifest.version -ne $appVersion) {
    throw "Manifest app version does not match app-version.json."
}

if ($writtenManifest.url -ne $updateUrl) {
    throw "Manifest app update URL is incorrect."
}

if ($writtenManifest.sha256 -ne $appSha256 -or
    $writtenManifest.sha256 -notmatch '^[a-f0-9]{64}$') {
    throw "Manifest app update SHA-256 verification failed."
}

if ($writtenManifest.vendor -ne $vendorVersion) {
    throw "Manifest vendor version does not match app-version.json."
}

if ($writtenManifest.vendorUrl -ne $vendorUrl) {
    throw "Manifest vendor URL is incorrect."
}

if ($writtenManifest.vendorSha256 -ne $vendorSha256 -or
    $writtenManifest.vendorSha256 -notmatch '^[a-f0-9]{64}$') {
    throw "Manifest vendor SHA-256 verification failed."
}

# Verify that the generated vendor version marker matches the config.
$writtenVendorVersion = Get-Content -LiteralPath `
    (Join-Path $release "vendor\$vendorVersionJsonName") -Raw |
    ConvertFrom-Json

if ($writtenVendorVersion.version -ne $vendorVersion) {
    throw "Vendor version marker does not match app-version.json."
}

# ------------------------------------------------------------
# 9. Build summary
# ------------------------------------------------------------

Write-Host ""
Write-Host "============================================"
Write-Host "CypherSR build completed successfully"
Write-Host "============================================"
Write-Host "App version:       $appVersion"
Write-Host "Vendor version:    $vendorVersion"
Write-Host ""
Write-Host "Launcher:          $releaseLauncher"
Write-Host "Core:              $(Join-Path $release $coreName)"
Write-Host "Full ZIP:          $fullZip"
Write-Host "App update ZIP:    $updateZip"
Write-Host "Vendor update ZIP: $vendorZip"
Write-Host "Manifest:          $manifestPath"
Write-Host ""
Write-Host "App update URL:    $updateUrl"
Write-Host "App SHA-256:       $appSha256"
Write-Host ""
Write-Host "Vendor update URL: $vendorUrl"
Write-Host "Vendor SHA-256:    $vendorSha256"
Write-Host ""
Write-Host "Upload all three ZIPs and latest.json to release $releaseTag."
Write-Host "App updates exclude vendor files and the launcher."
Write-Host "Vendor updates are packaged separately."
Write-Host "Existing release\data was preserved."
Write-Host "Neither ZIP contains the data folder."
Write-Host "============================================"
