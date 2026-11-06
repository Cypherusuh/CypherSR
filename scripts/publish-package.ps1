$ErrorActionPreference = "Stop"

$RepositoryRoot = Split-Path -Parent $PSScriptRoot
$DesktopRoot = Join-Path $RepositoryRoot "desktop"
$OutputRoot = $RepositoryRoot

$DesktopExeName = "CypherSR-Desktop.exe"
$PatcherExeName = "CypherSR-Updater.exe"
$CoreExeName = "CypherSR-Core.exe"
$BackendDirectoryName = "backend"
$MpvDirectoryName = "mpv"
$Repository = "Cypherusuh/CypherSR"
$FullFolderName = "CypherSR Desktop"

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

function Write-Utf8NoBom {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Path,

        [Parameter(Mandatory = $true)]
        [string]$Content
    )

    [System.IO.File]::WriteAllText(
        $Path,
        $Content,
        [System.Text.UTF8Encoding]::new($false)
    )
}

function Get-UrlFilename {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Url
    )

    $cleanUrl = ($Url -split '[?#]', 2)[0].Replace('\', '/')
    return [System.IO.Path]::GetFileName($cleanUrl)
}

function Get-DesktopVersion {
    param(
        [Parameter(Mandatory = $true)]
        [string]$VersionPath
    )

    try {
        $versionConfig = Get-Content -LiteralPath $VersionPath -Raw |
            ConvertFrom-Json
    }
    catch {
        throw "Failed to parse app-version.json: $($_.Exception.Message)"
    }

    $version = [string]$versionConfig.version

    if ($version -notmatch '^\d+\.\d+\.\d+$') {
        throw "app-version.json must contain version in the format 1.0.0"
    }

    return $version
}

function Find-ManifestZipProperty {
    param(
        [Parameter(Mandatory = $true)]
        $Manifest,

        [Parameter(Mandatory = $true)]
        [string]$Pattern,

        [Parameter(Mandatory = $true)]
        [string]$Description
    )

    $matches = @(
        $Manifest.PSObject.Properties |
            Where-Object {
                $_.Value -is [string] -and
                $_.Value -match '(?i)\.zip(?:[?#].*)?$' -and
                (
                    $_.Name -match $Pattern -or
                    (Get-UrlFilename -Url $_.Value) -match $Pattern
                )
            }
    )

    if ($matches.Count -ne 1) {
        throw "Expected exactly one $Description ZIP URL in latest.json; found $($matches.Count). Check the manifest schema and URLs."
    }

    return $matches[0]
}

function Find-SourceZip {
    param(
        [Parameter(Mandatory = $true)]
        [string]$BackendRelease,

        [Parameter(Mandatory = $true)]
        [string]$Filename,

        [Parameter(Mandatory = $true)]
        [string]$Description
    )

    $path = Join-Path $BackendRelease $Filename
    Require-Path -Path $path -Description $Description
    return $path
}

function Get-Sha256 {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Path
    )

    return (
        Get-FileHash -LiteralPath $Path -Algorithm SHA256
    ).Hash.ToLowerInvariant()
}

# ---------------------------------------------------------------------------
# 1. Validate build outputs and read versions.
# ---------------------------------------------------------------------------

$cargoManifest = Join-Path $DesktopRoot "Cargo.toml"
$versionFile = Join-Path $RepositoryRoot "app-version.json"
$desktopVersion = Get-DesktopVersion -VersionPath $versionFile
$releaseRoot = Join-Path $RepositoryRoot "release"
$versionPublishDir = Join-Path $releaseRoot "v$desktopVersion"
$versioningDir = Join-Path $versionPublishDir "versioning"
$publishedAppDir = Join-Path $versionPublishDir "Published App"
$publishingZipDir = Join-Path $versionPublishDir "Publishing zip"
$backendRelease = Join-Path $versionPublishDir "Backend Build"
$desktopRelease = Join-Path $versionPublishDir "Desktop Build"

$desktopExe = Join-Path $desktopRelease $DesktopExeName
$desktopConfig = Join-Path $DesktopRoot "config.toml"
$desktopGuiZipName = "CypherSR-Desktop-GUI-v$desktopVersion-win-x64.zip"
$desktopGuiZipSource = Join-Path $publishingZipDir $desktopGuiZipName
$desktopUpdateAssetName = "CypherSR-Desktop-GUI-v$desktopVersion-win-x64.exe"

$backendConfigPath = $versionFile
$backendManifestPath = Join-Path $backendRelease "latest.json"

$backendPatcher = Join-Path $backendRelease $PatcherExeName
$backendCore = Join-Path $backendRelease $CoreExeName
$backendPublic = Join-Path $backendRelease "public"
$backendVendor = Join-Path $backendRelease "vendor"
$mpvSource = Join-Path $backendVendor $MpvDirectoryName

foreach ($item in @(
    @{ Path = $cargoManifest; Description = "Desktop Cargo.toml" },
    @{ Path = $desktopExe; Description = "Built Desktop executable" },
    @{ Path = $desktopConfig; Description = "Desktop config.toml template" },
    @{ Path = (Join-Path $mpvSource "mpv.exe"); Description = "Bundled MPV executable" },
    @{ Path = (Join-Path $mpvSource "mpv-register.bat"); Description = "MPV registration script" },
    @{ Path = $desktopGuiZipSource; Description = "Versioned Desktop GUI ZIP" },
    @{ Path = $backendConfigPath; Description = "Backend app-version.json" },
    @{ Path = $backendManifestPath; Description = "Backend latest.json" },
    @{ Path = (Join-Path $versioningDir "desktop.json"); Description = "Desktop version metadata" },
    @{ Path = (Join-Path $versioningDir "backend.json"); Description = "Backend version metadata" },
    @{ Path = (Join-Path $versioningDir "vendor.json"); Description = "Vendor version metadata" },
    @{ Path = $backendPatcher; Description = "Built backend patcher" },
    @{ Path = $backendCore; Description = "Built backend core" },
    @{ Path = (Join-Path $backendPublic "player.html"); Description = "Backend public\player.html" },
    @{ Path = (Join-Path $backendVendor "vendor-version.json"); Description = "Backend vendor-version.json" },
    @{ Path = (Join-Path $backendVendor "yt-dlp.exe"); Description = "Backend yt-dlp.exe" },
    @{ Path = (Join-Path $backendVendor "ffmpeg\bin\ffmpeg.exe"); Description = "Backend ffmpeg.exe" }
)) {
    Require-Path -Path $item.Path -Description $item.Description
}

$backendConfig = Get-Content -LiteralPath $backendConfigPath -Raw |
    ConvertFrom-Json

$backendVersion = [string]$backendConfig.version
$vendorVersion = [string]$backendConfig.vendorVersion

if ($backendVersion -notmatch '^\d+\.\d+\.\d+$') {
    throw "Backend app-version.json has an invalid version."
}

if ($vendorVersion -notmatch '^\d+\.\d+\.\d+$') {
    throw "Backend app-version.json has an invalid vendorVersion."
}

if ($backendVersion -ne $desktopVersion) {
    throw "Desktop and backend versions must match the root app-version.json."
}

$desktopVersionMetadata = Get-Content -LiteralPath `
    (Join-Path $versioningDir "desktop.json") -Raw | ConvertFrom-Json
$backendVersionMetadata = Get-Content -LiteralPath `
    (Join-Path $versioningDir "backend.json") -Raw | ConvertFrom-Json
$vendorVersionMetadata = Get-Content -LiteralPath `
    (Join-Path $versioningDir "vendor.json") -Raw | ConvertFrom-Json

if ([string]$desktopVersionMetadata.version -ne $desktopVersion) {
    throw "Desktop version metadata does not match app-version.json."
}

if ([string]$backendVersionMetadata.version -ne $backendVersion) {
    throw "Backend version metadata does not match app-version.json."
}

if ([string]$vendorVersionMetadata.version -ne $vendorVersion) {
    throw "Vendor version metadata does not match app-version.json."
}

# The Lite ZIP uses the backend version, so define it only after reading
# and validating app-version.json.
$liteZipName = "CypherSR-Lite-v$backendVersion-win-x64.zip"
$liteZipSource = Join-Path $publishingZipDir $liteZipName

Require-Path -Path $liteZipSource -Description "Backend Lite ZIP"

$backendManifest = Get-Content -LiteralPath $backendManifestPath -Raw |
    ConvertFrom-Json

$vendorMarker = Get-Content -LiteralPath (Join-Path $backendVendor "vendor-version.json") -Raw |
    ConvertFrom-Json

if ([string]$backendManifest.version -ne $backendVersion) {
    throw "Backend version mismatch: app-version.json says $backendVersion, latest.json says $($backendManifest.version)."
}

if ([string]$backendManifest.vendor -ne $vendorVersion) {
    throw "Vendor version mismatch: app-version.json says $vendorVersion, latest.json says $($backendManifest.vendor)."
}

if ([string]$vendorMarker.version -ne $vendorVersion) {
    throw "Vendor marker mismatch: expected $vendorVersion, found $($vendorMarker.version)."
}

# Verify the Desktop GUI ZIP contains exactly the expected executable.
Add-Type -AssemblyName System.IO.Compression.FileSystem

$guiArchive = [System.IO.Compression.ZipFile]::OpenRead($desktopGuiZipSource)

try {
    $guiEntries = @(
        $guiArchive.Entries |
            Where-Object { -not [string]::IsNullOrEmpty($_.Name) } |
            ForEach-Object { $_.FullName.Replace("\", "/") }
    )

    if (
        $guiEntries.Count -ne 1 -or
        $guiEntries[0] -ne $DesktopExeName
    ) {
        throw "Desktop GUI ZIP must contain only '$DesktopExeName' at the archive root."
    }
}
finally {
    $guiArchive.Dispose()
}

# Find the backend update and vendor ZIPs from the backend manifest.
$backendUpdateUrlProperty = Find-ManifestZipProperty `
    -Manifest $backendManifest `
    -Pattern '(?i)(update-only|backend-update)' `
    -Description "backend update"

$vendorUrlProperty = Find-ManifestZipProperty `
    -Manifest $backendManifest `
    -Pattern '(?i)vendor' `
    -Description "vendor"

$backendUpdateSourceName = Get-UrlFilename -Url $backendUpdateUrlProperty.Value

$backendUpdateSource = Find-SourceZip `
    -BackendRelease $publishingZipDir `
    -Filename $backendUpdateSourceName `
    -Description "Backend update ZIP"

# ---------------------------------------------------------------------------
# 2. Define output layout and asset names.
# ---------------------------------------------------------------------------

$productDir = Join-Path $publishedAppDir $FullFolderName
$publishDir = $publishingZipDir
$stageDir = Join-Path $OutputRoot ".staging-$desktopVersion"

$fullZipName = "CypherSR-Full-v$desktopVersion-win-x64.zip"
$backendUpdateName = "CypherSR-Backend-Update-v$backendVersion-win-x64.zip"
$vendorZipName = "vendor-$vendorVersion-win-x64.zip"

$fullZipPath = Join-Path $publishingZipDir $fullZipName
$backendUpdatePath = Join-Path $publishingZipDir $backendUpdateName
$vendorZipPath = Join-Path $publishingZipDir $vendorZipName
$liteZipPath = Join-Path $publishingZipDir $liteZipName

# Desktop tags are distinct from backend v<version> tags.
$releaseTag = "desktop-v$desktopVersion"
$releaseDownloadBase = "https://github.com/$Repository/releases/download/$releaseTag"

$fullDownloadUrl = "$releaseDownloadBase/$fullZipName"
$desktopUpdateDownloadUrl = "$releaseDownloadBase/$desktopUpdateAssetName"
$backendUpdateDownloadUrl = "$releaseDownloadBase/$backendUpdateName"
$vendorDownloadUrl = "$releaseDownloadBase/$vendorZipName"
$liteDownloadUrl = "$releaseDownloadBase/$liteZipName"

Write-Host ""
Write-Host "============================================"
Write-Host "Packaging CypherSR"
Write-Host "============================================"
Write-Host "Desktop version: $desktopVersion"
Write-Host "Backend version: $backendVersion"
Write-Host "Vendor version:  $vendorVersion"
Write-Host "Product folder:  $productDir"
Write-Host "Published app:   $publishedAppDir"
Write-Host "Publishing ZIPs: $publishingZipDir"
Write-Host "Backend build:   $backendRelease"
Write-Host "Desktop build:   $desktopRelease"
Write-Host "GitHub tag:      $releaseTag"
Write-Host ""

# ---------------------------------------------------------------------------
# 3. Assemble the runnable product folder.
#
# Preserve existing data\. Remove backend\.tmp\ because the updater creates it
# when needed. Never copy backend release\data into the product.
# ---------------------------------------------------------------------------

Write-Host "Assembling product folder..."

New-Item -ItemType Directory -Path $productDir -Force | Out-Null

Copy-Item -LiteralPath $desktopExe `
    -Destination (Join-Path $productDir $DesktopExeName) -Force

if (-not (Test-Path -LiteralPath (Join-Path $productDir "config.toml"))) {
    Copy-Item -LiteralPath $desktopConfig `
        -Destination (Join-Path $productDir "config.toml") -Force
}

$productBackend = Join-Path $productDir $BackendDirectoryName
$productPublic = Join-Path $productBackend "public"
$productVendor = Join-Path $productBackend "vendor"
$productMpv = Join-Path $productVendor $MpvDirectoryName
$productTmp = Join-Path $productBackend ".tmp"
$legacyProductTmp = Join-Path $productDir ".tmp"
$legacyProductVersion = Join-Path $productDir "version.json"
$legacyProductPublic = Join-Path $productDir "public"
$legacyProductVendor = Join-Path $productDir "vendor"

Remove-Item -LiteralPath $productPublic, $productVendor, $productTmp, `
    $legacyProductPublic, $legacyProductVendor, $legacyProductTmp `
    -Recurse -Force -ErrorAction SilentlyContinue

if (Test-Path -LiteralPath $legacyProductVersion) {
    Remove-Item -LiteralPath $legacyProductVersion -Force
}

New-Item -ItemType Directory -Path $productBackend -Force | Out-Null

foreach ($fileName in @(
    $PatcherExeName,
    "Cypher-Updater.exe",
    $CoreExeName,
    "CypherSR.exe",
    "CypherSR-core.exe"
)) {
    $staleBackendExe = Join-Path $productBackend $fileName

    if (Test-Path -LiteralPath $staleBackendExe) {
        Remove-Item -LiteralPath $staleBackendExe -Force
    }
}

Copy-Item -LiteralPath $backendPatcher `
    -Destination (Join-Path $productBackend $PatcherExeName) -Force

Copy-Item -LiteralPath $backendCore `
    -Destination (Join-Path $productBackend $CoreExeName) -Force

foreach ($fileName in @(
    $PatcherExeName,
    "Cypher-Updater.exe",
    $CoreExeName,
    "CypherSR.exe",
    "CypherSR-core.exe"
)) {
    $staleRootExe = Join-Path $productDir $fileName

    if (Test-Path -LiteralPath $staleRootExe) {
        Remove-Item -LiteralPath $staleRootExe -Force
    }
}

Copy-Item -LiteralPath $backendPublic `
    -Destination $productPublic -Recurse -Force

Copy-Item -LiteralPath $backendVendor `
    -Destination $productVendor -Recurse -Force

Copy-Item -LiteralPath $mpvSource `
    -Destination $productVendor -Recurse -Force

# version.json describes the backend version, not the Desktop version.
Write-Utf8NoBom `
    -Path (Join-Path $productBackend "version.json") `
    -Content (@{ version = $backendVersion } | ConvertTo-Json)

# ---------------------------------------------------------------------------
# 4. Create an allowlisted staging directory for the full portable ZIP.
#
# Neither data\ nor .tmp\ is copied to staging.
# ---------------------------------------------------------------------------

Remove-Item -LiteralPath $stageDir `
    -Recurse -Force -ErrorAction SilentlyContinue

New-Item -ItemType Directory -Path $stageDir -Force | Out-Null

foreach ($fileName in @(
    $DesktopExeName
)) {
    Copy-Item -LiteralPath (Join-Path $productDir $fileName) `
        -Destination $stageDir -Force
}

Copy-Item -LiteralPath $desktopConfig `
    -Destination (Join-Path $stageDir "config.toml") -Force

Copy-Item -LiteralPath $productBackend `
    -Destination $stageDir -Recurse -Force

# ---------------------------------------------------------------------------
# 5. Rebuild publishing assets.
#
# Runnable files and release artifacts are separated into their own folders.
# latest.json and distributable ZIPs live under "Publishing zip".
# ---------------------------------------------------------------------------

New-Item -ItemType Directory -Path @(
    $publishedAppDir,
    $publishingZipDir,
    $backendRelease,
    $desktopRelease,
    $versioningDir
) -Force | Out-Null

# Publish the executable separately for the in-app updater.
Copy-Item -LiteralPath $desktopExe `
    -Destination (Join-Path $desktopRelease $desktopUpdateAssetName) -Force
Require-Path `
    -Path (Join-Path $desktopRelease $desktopUpdateAssetName) `
    -Description "Desktop updater executable asset"

# Rename the backend updater ZIP without changing its contents.
if ([IO.Path]::GetFullPath($backendUpdateSource) -ne [IO.Path]::GetFullPath($backendUpdatePath)) {
    Copy-Item -LiteralPath $backendUpdateSource -Destination $backendUpdatePath -Force
}

$vendorStageRoot = Join-Path $stageDir "vendor-package"
$vendorStageDir = Join-Path $vendorStageRoot "vendor"
$vendorZipTempPath = Join-Path $publishingZipDir ".tmp-$vendorZipName"

New-Item -ItemType Directory -Path $vendorStageRoot -Force | Out-Null
Copy-Item -LiteralPath $productVendor `
    -Destination $vendorStageRoot -Recurse -Force

try {
    if (Test-Path -LiteralPath $vendorZipTempPath) {
        Remove-Item -LiteralPath $vendorZipTempPath -Force
    }

    Compress-Archive `
        -Path $vendorStageDir `
        -DestinationPath $vendorZipTempPath `
        -CompressionLevel Optimal `
        -Force

    Require-Path -Path $vendorZipTempPath -Description "Temporary vendor ZIP"

    Move-Item -LiteralPath $vendorZipTempPath `
        -Destination $vendorZipPath -Force
}
finally {
    if (Test-Path -LiteralPath $vendorZipTempPath) {
        Remove-Item -LiteralPath $vendorZipTempPath -Force
    }
}

# Copy the standalone Lite package byte-for-byte.
if ([IO.Path]::GetFullPath($liteZipSource) -ne
    [IO.Path]::GetFullPath($liteZipPath)) {
    Copy-Item -LiteralPath $liteZipSource `
        -Destination $liteZipPath -Force
}

# Create the full portable package.
# The temporary filename still ends with .zip for Compress-Archive.
# Create the full portable package with a parent folder.
$fullZipTempPath = Join-Path $publishingZipDir ".tmp-$fullZipName"
$fullStageRoot = Join-Path $stageDir $FullFolderName

try {
    if (Test-Path -LiteralPath $fullZipTempPath) {
        Remove-Item -LiteralPath $fullZipTempPath -Force
    }

    New-Item -ItemType Directory -Path $fullStageRoot -Force | Out-Null

    foreach ($fileName in @($DesktopExeName, "config.toml")) {
        Copy-Item -LiteralPath (Join-Path $stageDir $fileName) `
            -Destination $fullStageRoot -Force
    }

    Copy-Item -LiteralPath (Join-Path $stageDir $BackendDirectoryName) `
        -Destination $fullStageRoot -Recurse -Force

    Compress-Archive `
        -Path $fullStageRoot `
        -DestinationPath $fullZipTempPath `
        -CompressionLevel Optimal `
        -Force

    Require-Path -Path $fullZipTempPath -Description "Temporary full portable ZIP"

    Move-Item -LiteralPath $fullZipTempPath `
        -Destination $fullZipPath -Force
}
finally {
    if (Test-Path -LiteralPath $fullZipTempPath) {
        Remove-Item -LiteralPath $fullZipTempPath -Force
    }
}

Require-Path -Path $fullZipPath -Description "Full portable ZIP"
Require-Path -Path (Join-Path $productBackend $PatcherExeName) -Description "Packaged backend patcher"
Require-Path -Path (Join-Path $productBackend $CoreExeName) -Description "Packaged backend core"
Require-Path -Path (Join-Path $productDir "config.toml") -Description "Packaged config.toml"
Require-Path -Path (Join-Path $productMpv "mpv.exe") -Description "Packaged MPV executable"
Require-Path -Path (Join-Path $productMpv "mpv-register.bat") -Description "Packaged MPV registration script"

# ---------------------------------------------------------------------------
# 6. Merge updater metadata.
#
# Preserve the backend manifest schema and existing fields. Redirect backend
# and vendor URLs to assets published under the Desktop release tag.
# ---------------------------------------------------------------------------

$desktopUpdateHash = Get-Sha256 `
    -Path (Join-Path $desktopRelease $desktopUpdateAssetName)
$backendUpdateHash = Get-Sha256 -Path $backendUpdatePath
$vendorHash = Get-Sha256 -Path $vendorZipPath
$fullHash = Get-Sha256 -Path $fullZipPath
$liteHash = Get-Sha256 -Path $liteZipPath

# Verify that the Lite ZIP was copied without changing its contents.
$liteSourceHash = Get-Sha256 -Path $liteZipSource

if ($liteHash -ne $liteSourceHash) {
    throw "Lite ZIP hash mismatch: the published copy differs from the backend source."
}

$backendManifest.PSObject.Properties[
    $backendUpdateUrlProperty.Name
].Value = $backendUpdateDownloadUrl

$backendManifest.PSObject.Properties[
    $vendorUrlProperty.Name
].Value = $vendorDownloadUrl

# Refresh existing hash fields, if provided by the backend manifest.
foreach ($property in $backendManifest.PSObject.Properties) {
    if (
        $property.Name -match '(?i)(sha256|hash)' -and
        $property.Value -is [string]
    ) {
        if ($property.Name -match '(?i)vendor') {
            $property.Value = $vendorHash
        }
        elseif (
            $property.Name -match '(?i)(app|backend|update|patcher)' -or
            $property.Name -eq "sha256"
        ) {
            $property.Value = $backendUpdateHash
        }
    }
}

$desktopEntry = [PSCustomObject]@{
    version = $desktopVersion
    url     = $desktopUpdateDownloadUrl
    sha256  = $desktopUpdateHash
}

$fullEntry = [PSCustomObject]@{
    version = $desktopVersion
    url     = $fullDownloadUrl
    sha256  = $fullHash
}

$liteEntry = [PSCustomObject]@{
    version = $backendVersion
    url     = $liteDownloadUrl
    sha256  = $liteHash
}

$backendManifest |
    Add-Member -MemberType NoteProperty -Name "desktop" `
        -Value $desktopEntry -Force

$backendManifest |
    Add-Member -MemberType NoteProperty -Name "full" `
        -Value $fullEntry -Force

$backendManifest |
    Add-Member -MemberType NoteProperty -Name "lite" `
        -Value $liteEntry -Force

$manifestOutput = Join-Path $publishDir "latest.json"
$manifestJson = $backendManifest | ConvertTo-Json -Depth 20

Write-Utf8NoBom -Path $manifestOutput -Content $manifestJson

# ---------------------------------------------------------------------------
# 7. Validate generated files and archive contents.
# ---------------------------------------------------------------------------
if (Test-Path -LiteralPath $productTmp) {
    throw "The product folder must not contain .tmp."
}

# Ensure the full ZIP contains no data\ or .tmp\ entries,
# and that all expected files are inside the parent folder.
$fullArchive = [System.IO.Compression.ZipFile]::OpenRead($fullZipPath)

try {
    $fullEntries = @(
        $fullArchive.Entries | ForEach-Object {
            $_.FullName.Replace("\", "/")
        }
    )

    # The full ZIP must not contain persistent data or temporary files.
    foreach ($entryName in $fullEntries) {
        if ($entryName -match '(^|/)(data|\.tmp)(/|$)') {
            throw "Forbidden path found in full ZIP: $entryName"
        }
    }

    # All files must be inside the "CypherSR Desktop" folder.
    $archiveRoot = "CypherSR Desktop/"

    foreach ($expectedEntry in @(
        "${archiveRoot}$DesktopExeName",
        "${archiveRoot}config.toml",
        "${archiveRoot}${BackendDirectoryName}/$PatcherExeName",
        "${archiveRoot}${BackendDirectoryName}/$CoreExeName",
        "${archiveRoot}${BackendDirectoryName}/version.json",
        "${archiveRoot}${BackendDirectoryName}/public/player.html",
        "${archiveRoot}${BackendDirectoryName}/vendor/vendor-version.json",
        "${archiveRoot}${BackendDirectoryName}/vendor/yt-dlp.exe",
        "${archiveRoot}${BackendDirectoryName}/vendor/ffmpeg/bin/ffmpeg.exe",
        "${archiveRoot}${BackendDirectoryName}/vendor/mpv/mpv.exe",
        "${archiveRoot}${BackendDirectoryName}/vendor/mpv/mpv-register.bat"
    )) {
        if ($fullEntries -notcontains $expectedEntry) {
            throw "Expected file missing from full ZIP: $expectedEntry"
        }
    }

    # Ensure no files are accidentally exposed at the ZIP root.
    foreach ($entryName in $fullEntries) {
        if (
            -not $entryName.StartsWith($archiveRoot) -and
            -not [string]::IsNullOrEmpty(
                [System.IO.Path]::GetFileName($entryName)
            )
        ) {
            throw "Unexpected file outside '$archiveRoot': $entryName"
        }
    }
}
finally {
    $fullArchive.Dispose()
}

$vendorArchive = [System.IO.Compression.ZipFile]::OpenRead($vendorZipPath)

try {
    $vendorEntries = @(
        $vendorArchive.Entries | ForEach-Object {
            $_.FullName.Replace("\", "/")
        }
    )

    foreach ($expectedEntry in @(
        "vendor/vendor-version.json",
        "vendor/yt-dlp.exe",
        "vendor/ffmpeg/bin/ffmpeg.exe",
        "vendor/mpv/mpv.exe",
        "vendor/mpv/mpv-register.bat"
    )) {
        if ($vendorEntries -notcontains $expectedEntry) {
            throw "Expected file missing from vendor ZIP: $expectedEntry"
        }
    }

    foreach ($entryName in $vendorEntries) {
        if (
            -not $entryName.StartsWith("vendor/") -and
            -not [string]::IsNullOrEmpty(
                [System.IO.Path]::GetFileName($entryName)
            )
        ) {
            throw "Unexpected file outside 'vendor/': $entryName"
        }
    }
}
finally {
    $vendorArchive.Dispose()
}

# Confirm the merged manifest matches the generated assets.
$verifiedManifest = Get-Content -LiteralPath $manifestOutput -Raw |
    ConvertFrom-Json

if (
    [string]$verifiedManifest.desktop.version -ne $desktopVersion -or
    [string]$verifiedManifest.desktop.url -ne $desktopUpdateDownloadUrl -or
    [string]$verifiedManifest.desktop.sha256 -ne $desktopUpdateHash -or

    [string]$verifiedManifest.full.version -ne $desktopVersion -or
    [string]$verifiedManifest.full.url -ne $fullDownloadUrl -or
    [string]$verifiedManifest.full.sha256 -ne $fullHash -or

    [string]$verifiedManifest.lite.version -ne $backendVersion -or
    [string]$verifiedManifest.lite.url -ne $liteDownloadUrl -or
    [string]$verifiedManifest.lite.sha256 -ne $liteHash -or

    [string]$verifiedManifest.version -ne $backendVersion -or
    [string]$verifiedManifest.vendor -ne $vendorVersion
) {
    throw "Merged latest.json verification failed."
}

# Clean temporary packaging files. Never delete product data.
Remove-Item -LiteralPath $stageDir `
    -Recurse -Force -ErrorAction SilentlyContinue

Write-Host ""
Write-Host "============================================"
Write-Host "Packaging completed successfully"
Write-Host "============================================"
Write-Host "Runnable product: $productDir"
Write-Host "Published app:    $publishedAppDir"
Write-Host "Publishing ZIPs:  $publishingZipDir"
Write-Host "Backend build:    $backendRelease"
Write-Host "Desktop build:    $desktopRelease"
Write-Host "Manifest:         $manifestOutput"
Write-Host "Backend manifest: $backendManifestPath"
Write-Host ""
Write-Host "Full release:     $fullZipName"
Write-Host "Desktop GUI ZIP:  $desktopGuiZipName"
Write-Host "Backend updater:  $backendUpdateName"
Write-Host "Vendor package:   $vendorZipName"
Write-Host "Lite package:     $liteZipName"
Write-Host ""
Write-Host "Desktop update EXE: $desktopUpdateAssetName"
Write-Host "Product data\ was preserved."
Write-Host "Product backend\.tmp\ was removed."
Write-Host "Full ZIP excludes data\ and .tmp\."
Write-Host "Backend release\data was not copied."
Write-Host "Lite ZIP was copied unchanged."
Write-Host ""
Write-Host "GitHub release tag: $releaseTag"
Write-Host "Review the release assets before publishing."
Write-Host "============================================"
