
$ErrorActionPreference = "Stop"

$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

$releaseRoot = Join-Path $root "release"
$build = Join-Path $root "dist-sea"
$packageRoot = Join-Path $build "packages"

$versionFile = Join-Path $root "app-version.json"
$publicSource = Join-Path $root "public"
$ytDlpSource = Join-Path $root "vendor\yt-dlp.exe"
$ffmpegSource = Join-Path $root "vendor\ffmpeg\bin\ffmpeg.exe"
$defaultMpvSource = Join-Path $root "vendor\mpv"
$mpvSource = if ($env:CYPHERSR_MPV_SOURCE) {
    $env:CYPHERSR_MPV_SOURCE
} else {
    $defaultMpvSource
}

$cargoManifest = Join-Path $root "launcher-rust\Cargo.toml"
$rustLauncher = Join-Path $root "launcher-rust\target\release\CypherSR-Updater.exe"

$repository = "Cypherusuh/CypherSR"

$launcherName = "CypherSR-Updater.exe"
$coreName = "CypherSR-Core.exe"
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

$versionRoot = Join-Path $releaseRoot "v$appVersion"
$release = Join-Path $versionRoot "Backend Build"
$publishingZip = Join-Path $versionRoot "Publishing zip"
$versioning = Join-Path $versionRoot "versioning"

# Backend and Desktop share the app version; the vendor version is independent.
#
# Standalone backend release:
#   Tag: v{BackendVersion}
#   CypherSR-Backend-Update-v{BackendVersion}-win-x64.zip
#   vendor-{VendorVersion}-win-x64.zip
#
# Combined Desktop releases use the same v{BackendVersion} tag and asset URLs.
$updateAssetName = "CypherSR-Backend-Update-v$appVersion-win-x64.zip"
$vendorAssetName = "vendor-$vendorVersion-win-x64.zip"
$fullAssetName = "CypherSR-Lite-v$appVersion-win-x64.zip"

$releaseTag = "v$appVersion"
$releaseBaseUrl = "https://github.com/$repository/releases/download/$releaseTag"

$updateUrl = "$releaseBaseUrl/$updateAssetName"
$vendorUrl = "$releaseBaseUrl/$vendorAssetName"

foreach ($path in @(
    $publicSource,
    $ytDlpSource,
    $ffmpegSource,
    (Join-Path $mpvSource "mpv.exe"),
    (Join-Path $mpvSource "mpv-register.bat"),
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
    $releaseRoot, $release, $publishingZip, $versioning, $build, $packageRoot |
    Out-Null

# Preserve user data. Never clean or package release\data.
$dataDirectory = Join-Path $releaseRoot "data"

New-Item -ItemType Directory -Force -Path $dataDirectory |
    Out-Null

Write-Host "CypherSR backend version:  $appVersion"
Write-Host "Vendor version:            $vendorVersion"
Write-Host "Standalone release tag:    $releaseTag"
Write-Host "Backend update asset:      $updateAssetName"
Write-Host "Vendor asset:              $vendorAssetName"

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

    $esbuildExternalArgs = @(
        '--external:@napi-rs/keyring',
        '--external:@napi-rs/keyring-win32-x64-msvc'
    )

    npx esbuild $entryPath `
        --bundle `
        --platform=node `
        --format=cjs `
        --target=node24 `
        --log-override:empty-import-meta=error `
        @esbuildExternalArgs `
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

$legacyRustLauncher = Join-Path $root "launcher-rust\target\release\CypherSR.exe"
if (Test-Path -LiteralPath $legacyRustLauncher) {
    Remove-Item -LiteralPath $legacyRustLauncher -Force
}

cargo build --release --manifest-path $cargoManifest

if ($LASTEXITCODE -ne 0) {
    throw "Rust launcher build failed."
}

if (-not (Test-Path -LiteralPath $rustLauncher -PathType Leaf)) {
    throw "Rust launcher executable not found: $rustLauncher"
}

$releaseLauncher = Join-Path $release $launcherName
$legacyLauncher = Join-Path $release "Cypher-Updater.exe"

if (Test-Path -LiteralPath $legacyLauncher) {
    Remove-Item -LiteralPath $legacyLauncher -Force
}

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
$releaseNodeModules = Join-Path $release "node_modules"
Remove-Item -LiteralPath @(
    $releasePublic,
    $releaseVendor,
    $releaseNodeModules
) -Recurse -Force -ErrorAction SilentlyContinue

Copy-Item -LiteralPath $publicSource `
    -Destination $releasePublic -Recurse -Force

# Native addons required at runtime by the bundled SEA executable (e.g. @napi-rs/keyring).
# These must exist on disk alongside the executable and remain external to the esbuild bundle.
$nativeRuntimePackages = @(
    "@napi-rs\keyring",
    "@napi-rs\keyring-win32-x64-msvc"
)

foreach ($pkg in $nativeRuntimePackages) {
    $sourcePkg = Join-Path $root "node_modules\$pkg"
    $destPkg = Join-Path $release "node_modules\$pkg"

    if (-not (Test-Path -LiteralPath $sourcePkg)) {
        throw "Required native runtime package not found: $sourcePkg"
    }

    New-Item -ItemType Directory -Force -Path (Split-Path -Parent $destPkg) | Out-Null
    Copy-Item -LiteralPath $sourcePkg -Destination $destPkg -Recurse -Force
    Write-Host "Copied native runtime package: $pkg"
}

$releaseYtDlp = Join-Path $releaseVendor "yt-dlp.exe"
$releaseFfmpegDir = Join-Path $releaseVendor "ffmpeg\bin"

New-Item -ItemType Directory -Force -Path `
    (Split-Path -Parent $releaseYtDlp),
    $releaseFfmpegDir | Out-Null

Copy-Item -LiteralPath $ytDlpSource `
    -Destination $releaseYtDlp -Force

Copy-Item -LiteralPath $ffmpegSource `
    -Destination (Join-Path $releaseFfmpegDir "ffmpeg.exe") -Force

Copy-Item -LiteralPath $mpvSource `
    -Destination $releaseVendor -Recurse -Force

# Record the installed vendor version inside the vendor directory.
$vendorVersionJson = @{
    version = $vendorVersion
} | ConvertTo-Json

[System.IO.File]::WriteAllText(
    (Join-Path $releaseVendor $vendorVersionJsonName),
    $vendorVersionJson,
    [System.Text.UTF8Encoding]::new($false)
)

$backendVersionJson = @{
    version = $appVersion
} | ConvertTo-Json

[System.IO.File]::WriteAllText(
    (Join-Path $versioning "backend.json"),
    $backendVersionJson,
    [System.Text.UTF8Encoding]::new($false)
)

[System.IO.File]::WriteAllText(
    (Join-Path $versioning "vendor.json"),
    $vendorVersionJson,
    [System.Text.UTF8Encoding]::new($false)
)
# Unified versioning manifest for future consumers (kept alongside legacy folder).
$versioningManifestPath = Join-Path $versionRoot "versioning.json"
$vComponents = [ordered]@{}
if (Test-Path -LiteralPath $versioningManifestPath) {
    try {
        $existing = Get-Content -LiteralPath $versioningManifestPath -Raw | ConvertFrom-Json
        if ($existing.components) {
            foreach ($p in $existing.components.PSObject.Properties) { $vComponents[$p.Name] = $p.Value }
        }
    } catch { }
}
$vComponents["backend"] = [ordered]@{ version = $appVersion }
$vComponents["vendor"]  = [ordered]@{ version = $vendorVersion }
[System.IO.File]::WriteAllText(
    $versioningManifestPath,
    ([ordered]@{ schemaVersion = 1; components = $vComponents } | ConvertTo-Json -Depth 20),
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
#   CypherSR/
#     CypherSR-Updater.exe
#     CypherSR-Core.exe
#     version.json
#     public/
#     vendor/
#
# Backend update ZIP:
#   CypherSR-Core.exe
#   version.json
#   public/
#
# Vendor ZIP:
#   vendor/
#     yt-dlp.exe
#     ffmpeg/bin/ffmpeg.exe
#     mpv/
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

    # Include native runtime packages required by the bundled core executable.
    $releaseNodeModules = Join-Path $release "node_modules"
    if (Test-Path -LiteralPath $releaseNodeModules) {
        Copy-Item -LiteralPath $releaseNodeModules `
            -Destination $Destination -Recurse -Force
    }
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
# Vendor files are excluded to keep the backend package slimmer; the launcher
# downloads them independently when missing or when the vendor version mismatches.
$fullAppStage = Join-Path $fullStage "CypherSR"

New-Item -ItemType Directory -Path $fullAppStage -Force |
    Out-Null

Copy-AppUpdatePayload -Destination $fullAppStage

Copy-Item -LiteralPath (Join-Path $release $launcherName) `
    -Destination $fullAppStage -Force

# Backend-only update: deliberately excludes vendor and launcher.
Copy-AppUpdatePayload -Destination $appUpdateStage

# Vendor update: archive contains a vendor/ directory at its root.
Copy-Item -LiteralPath (Join-Path $release "vendor") `
    -Destination $vendorStage -Recurse -Force

$fullZip = Join-Path $publishingZip $fullAssetName
$updateZip = Join-Path $publishingZip $updateAssetName
$vendorZip = Join-Path $publishingZip $vendorAssetName

Compress-StageZip -Stage $fullStage -Destination $fullZip
Compress-StageZip -Stage $appUpdateStage -Destination $updateZip
Compress-StageZip -Stage $vendorStage -Destination $vendorZip

# ------------------------------------------------------------
# 7. Generate standalone latest.json
#
# The standalone backend manifest points to release v{appVersion}.
# The Desktop publisher should generate its own merged manifest and
# use the Desktop release tag for assets uploaded to that release.
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
    (Join-Path $release "vendor\mpv\mpv.exe"),
    (Join-Path $release "vendor\mpv\mpv-register.bat"),
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
    throw "Manifest backend update URL is incorrect."
}

if ($writtenManifest.sha256 -ne $appSha256 -or
    $writtenManifest.sha256 -notmatch '^[a-f0-9]{64}$') {
    throw "Manifest backend update SHA-256 verification failed."
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

# Verify the generated vendor version marker.
$writtenVendorVersion = Get-Content -LiteralPath `
    (Join-Path $release "vendor\$vendorVersionJsonName") -Raw |
    ConvertFrom-Json

if ($writtenVendorVersion.version -ne $vendorVersion) {
    throw "Vendor version marker does not match app-version.json."
}

# Verify the standalone URLs use the backend release tag and filenames.
$expectedReleaseBaseUrl = "https://github.com/$repository/releases/download/v$appVersion"

if ($writtenManifest.url -ne "$expectedReleaseBaseUrl/$updateAssetName") {
    throw "Backend update URL does not match the expected standalone release URL."
}

if ($writtenManifest.vendorUrl -ne "$expectedReleaseBaseUrl/$vendorAssetName") {
    throw "Vendor URL does not match the expected standalone release URL."
}

# ------------------------------------------------------------
# 9. Build summary
# ------------------------------------------------------------

Write-Host ""
Write-Host "============================================"
Write-Host "CypherSR backend build completed successfully"
Write-Host "============================================"
Write-Host "Backend version:    $appVersion"
Write-Host "Vendor version:     $vendorVersion"
Write-Host "Standalone tag:     $releaseTag"
Write-Host ""
Write-Host "Launcher:           $releaseLauncher"
Write-Host "Core:               $(Join-Path $release $coreName)"
Write-Host "Full ZIP:           $fullZip"
Write-Host "Backend update ZIP: $updateZip"
Write-Host "Vendor update ZIP:  $vendorZip"
Write-Host "Manifest:           $manifestPath"
Write-Host ""
Write-Host "Backend update URL: $updateUrl"
Write-Host "Backend SHA-256:    $appSha256"
Write-Host ""
Write-Host "Vendor update URL:  $vendorUrl"
Write-Host "Vendor SHA-256:     $vendorSha256"
Write-Host ""
Write-Host "Upload ZIPs from $publishingZip and latest.json from $release to release $releaseTag."
Write-Host "Backend updates exclude vendor files and the launcher."
Write-Host "Vendor updates are packaged separately."
Write-Host "Existing release\data was preserved."
Write-Host "Neither ZIP contains the data folder."
Write-Host "============================================"
