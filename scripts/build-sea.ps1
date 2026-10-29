
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

$repository = "Cypherusuh/CypherSR"
$updateAssetName = "CypherSR-update-only-win-x64.zip"

$launcherName = "CypherSR.exe"
$coreName = "CypherSR-core.exe"
$versionJsonName = "version.json"

# Validate build inputs.
if (-not (Test-Path -LiteralPath $versionFile -PathType Leaf)) {
    throw "Missing app-version.json"
}

$versionConfig = Get-Content -LiteralPath $versionFile -Raw |
    ConvertFrom-Json

$appVersion = [string]$versionConfig.version

if ($appVersion -notmatch '^\d+\.\d+\.\d+$') {
    throw "app-version.json must contain a version like 1.0.0"
}

foreach ($path in @($publicSource, $ytDlpSource, $ffmpegSource)) {
    if (-not (Test-Path -LiteralPath $path)) {
        throw "Required build input not found: $path"
    }
}

if (-not (Get-Command node -ErrorAction SilentlyContinue)) {
    throw "Node.js is required to build the SEA executables."
}

if (-not (Get-Command npx -ErrorAction SilentlyContinue)) {
    throw "npx is required to build the SEA executables."
}

New-Item -ItemType Directory -Force -Path `
    $release, $build, $packageRoot | Out-Null

# Preserve user data; never include it in release packages.
New-Item -ItemType Directory -Force -Path `
    (Join-Path $release "data") | Out-Null

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

    Remove-Item -LiteralPath $bundle, $blob, $configPath, $temporaryExe `
        -Force -ErrorAction SilentlyContinue

    Write-Host ""
    Write-Host "Bundling $EntryPoint -> $OutputName"

    # SEA requires a CommonJS bundle.
    # Treat import.meta incompatibilities as errors to prevent this warning
    # from silently returning in future builds.
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

    # Build and inject into a temporary executable first.
    # The existing release executable remains untouched until success.
    Copy-Item -LiteralPath $nodeExe -Destination $temporaryExe -Force

    npx postject $temporaryExe NODE_SEA_BLOB $blob `
        --sentinel-fuse NODE_SEA_FUSE_fce680ab2cc467b6e072b8b5df1996b2

    if ($LASTEXITCODE -ne 0) {
        Remove-Item -LiteralPath $temporaryExe -Force `
            -ErrorAction SilentlyContinue
        throw "SEA injection failed for $OutputName"
    }

    if (-not (Test-Path -LiteralPath $temporaryExe -PathType Leaf)) {
        throw "Build output was not created: $temporaryExe"
    }

    # This may fail if the destination executable is still running.
    # In that case, close CypherSR before rebuilding.
    Move-Item -LiteralPath $temporaryExe -Destination $outputExe -Force

    Write-Host "Created $outputExe"
}

# Build the core and stable launcher.
Build-SeaExecutable `
    -EntryPoint "src/main.ts" `
    -OutputName $coreName

Build-SeaExecutable `
    -EntryPoint "src/launcher.ts" `
    -OutputName $launcherName

# Refresh the visible runtime assets without deleting release\data.
Write-Host ""
Write-Host "Copying application assets..."

$releasePublic = Join-Path $release "public"
$releaseVendor = Join-Path $release "vendor"

Remove-Item -LiteralPath $releasePublic, $releaseVendor `
    -Recurse -Force -ErrorAction SilentlyContinue

Copy-Item -LiteralPath $publicSource `
    -Destination $releasePublic -Recurse -Force

$releaseYtDlp = Join-Path $releaseVendor "yt-dlp.exe"
$releaseFfmpegDir = Join-Path $releaseVendor "ffmpeg\bin"

New-Item -ItemType Directory -Force -Path `
    (Split-Path -Parent $releaseYtDlp), $releaseFfmpegDir |
    Out-Null

Copy-Item -LiteralPath $ytDlpSource `
    -Destination $releaseYtDlp -Force

Copy-Item -LiteralPath $ffmpegSource `
    -Destination (Join-Path $releaseFfmpegDir "ffmpeg.exe") -Force

# Write the version used by the launcher to determine whether to update.
$versionJson = @{
    version = $appVersion
} | ConvertTo-Json

[System.IO.File]::WriteAllText(
    (Join-Path $release $versionJsonName),
    $versionJson,
    [System.Text.UTF8Encoding]::new($false)
)

function Copy-UpdatePayload {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Destination
    )

    New-Item -ItemType Directory -Path $Destination -Force | Out-Null

    # Intentionally exclude CypherSR.exe and latest.json.
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

    Remove-Item -LiteralPath $Destination -Force `
        -ErrorAction SilentlyContinue

    # Archive the stage's contents, not the stage directory itself.
    Compress-Archive `
        -Path (Join-Path $Stage "*") `
        -DestinationPath $Destination `
        -CompressionLevel Optimal

    if (-not (Test-Path -LiteralPath $Destination -PathType Leaf)) {
        throw "Failed to create ZIP: $Destination"
    }
}

# Start with clean staging directories to avoid stale package files.
$fullStage = Join-Path $packageRoot "full"
$updateStage = Join-Path $packageRoot "update"

foreach ($stage in @($fullStage, $updateStage)) {
    Remove-Item -LiteralPath $stage -Recurse -Force `
        -ErrorAction SilentlyContinue

    New-Item -ItemType Directory -Path $stage -Force | Out-Null
}

# Full portable package:
# CypherSR\<launcher, core, version, public, vendor>
$fullAppStage = Join-Path $fullStage "CypherSR"
New-Item -ItemType Directory -Path $fullAppStage -Force | Out-Null

Copy-UpdatePayload -Destination $fullAppStage

Copy-Item -LiteralPath (Join-Path $release $launcherName) `
    -Destination $fullAppStage -Force

# Automatic update package:
# ZIP root contains only core, version, public, and vendor.
Copy-UpdatePayload -Destination $updateStage

$fullZip = Join-Path $release "CypherSR-$appVersion-win-x64.zip"
$updateZip = Join-Path $release $updateAssetName

Compress-StageZip -Stage $fullStage -Destination $fullZip
Compress-StageZip -Stage $updateStage -Destination $updateZip

# Generate the manifest as a separate release asset.
$sha256 = (Get-FileHash -LiteralPath $updateZip -Algorithm SHA256).
    Hash.ToLowerInvariant()

$manifest = @{
    version = $appVersion
    url = "https://github.com/$repository/releases/latest/download/$updateAssetName"
    sha256 = $sha256
} | ConvertTo-Json

$manifestPath = Join-Path $release "latest.json"

[System.IO.File]::WriteAllText(
    $manifestPath,
    $manifest,
    [System.Text.UTF8Encoding]::new($false)
)

# Verify required release files exist before reporting success.
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

Write-Host ""
Write-Host "Build complete."
Write-Host "App version: $appVersion"
Write-Host "Launcher:    $(Join-Path $release $launcherName)"
Write-Host "Core:        $(Join-Path $release $coreName)"
Write-Host "Full ZIP:    $fullZip"
Write-Host "Update ZIP:  $updateZip"
Write-Host "Manifest:    $manifestPath"
Write-Host "SHA-256:     $sha256"
Write-Host ""
Write-Host "The automatic update ZIP excludes the stable launcher."
Write-Host "latest.json is a separate GitHub release asset."
Write-Host "Existing release\data was preserved."
Write-Host "Neither ZIP contains the data folder."
