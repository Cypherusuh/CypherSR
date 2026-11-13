$ErrorActionPreference = "Stop"

$Repository = "Cypherusuh/CypherSR"
$RepositoryRoot = Split-Path -Parent $PSScriptRoot
$versionConfigPath = Join-Path $RepositoryRoot "app-version.json"
$versionConfig = Get-Content -LiteralPath $versionConfigPath -Raw |
    ConvertFrom-Json
$version = [string]$versionConfig.version

if ($version -notmatch '^\d+\.\d+\.\d+$') {
    throw "app-version.json must contain a version in the format 1.0.0."
}

$tag = "v$version"
$apiBase = "https://api.github.com/repos/$Repository"
$headers = @{
    Accept = "application/vnd.github+json"
    "X-GitHub-Api-Version" = "2022-11-28"
    "User-Agent" = "CypherSR-release-verifier"
}

$release = Invoke-RestMethod `
    -Uri "$apiBase/releases/tags/$tag" `
    -Headers $headers

if ($release.draft -or $release.prerelease) {
    throw "Release $tag must be published and not marked as a prerelease."
}

$latest = Invoke-RestMethod `
    -Uri "$apiBase/releases/latest" `
    -Headers $headers

if ($latest.tag_name -ne $tag) {
    throw "Release $tag is not GitHub's latest release; latest is $($latest.tag_name). The backend updater uses latest.json."
}

$desktopUpdateName = "CypherSR-Desktop-GUI-v$version-win-x64.zip"
$desktopTransitionExeName = "CypherSR-Desktop-GUI-v$version-win-x64.exe"
$requiredAssets = @(
    $desktopUpdateName,
    $desktopTransitionExeName,
    "CypherSR-Backend-Update-v$version-win-x64.zip",
    "CypherSR-Full-v$version-win-x64.zip",
    "CypherSR-Lite-v$version-win-x64.zip",
    "vendor-$($versionConfig.vendorVersion)-win-x64.zip",
    "latest.json"
)

$assetByName = @{}
foreach ($asset in $release.assets) {
    if ($assetByName.ContainsKey($asset.name)) {
        throw "Release $tag has duplicate asset name '$($asset.name)'."
    }

    $assetByName[$asset.name] = $asset
}

foreach ($name in $requiredAssets) {
    if (-not $assetByName.ContainsKey($name)) {
        throw "Release $tag is missing required asset '$name'."
    }

    if ($assetByName[$name].size -le 0) {
        throw "Release asset '$name' is empty."
    }
}

$manifestUrl = $assetByName["latest.json"].browser_download_url
$manifest = Invoke-RestMethod -Uri $manifestUrl -Headers $headers
$releaseBaseUrl = "https://github.com/$Repository/releases/download/$tag"

$expectedManifestAssets = @(
    @{
        Name = $desktopUpdateName
        Url = [string]$manifest.desktop.url
        Hash = [string]$manifest.desktop.sha256
    },
    @{
        Name = "CypherSR-Backend-Update-v$version-win-x64.zip"
        Url = [string]$manifest.url
        Hash = [string]$manifest.sha256
    },
    @{
        Name = "vendor-$($versionConfig.vendorVersion)-win-x64.zip"
        Url = [string]$manifest.vendorUrl
        Hash = [string]$manifest.vendorSha256
    },
    @{
        Name = "CypherSR-Full-v$version-win-x64.zip"
        Url = [string]$manifest.full.url
        Hash = [string]$manifest.full.sha256
    },
    @{
        Name = "CypherSR-Lite-v$version-win-x64.zip"
        Url = [string]$manifest.lite.url
        Hash = [string]$manifest.lite.sha256
    }
)

if (
    [string]$manifest.version -ne $version -or
    [string]$manifest.desktop.version -ne $version -or
    [string]$manifest.full.version -ne $version -or
    [string]$manifest.lite.version -ne $version -or
    [string]$manifest.vendor -ne [string]$versionConfig.vendorVersion
) {
    throw "latest.json versions do not match app-version.json."
}

foreach ($entry in $expectedManifestAssets) {
    $expectedUrl = "$releaseBaseUrl/$($entry.Name)"
    if ($entry.Url -ne $expectedUrl) {
        throw "latest.json URL for '$($entry.Name)' is incorrect: $($entry.Url)"
    }

    if ($entry.Hash -notmatch '^[0-9a-fA-F]{64}$') {
        throw "latest.json has an invalid SHA-256 for '$($entry.Name)'."
    }

    $asset = $assetByName[$entry.Name]
    if (
        $asset.digest -and
        $asset.digest -ne "sha256:$($entry.Hash.ToLowerInvariant())"
    ) {
        throw "SHA-256 mismatch between latest.json and GitHub asset '$($entry.Name)'."
    }
}

Write-Host "Release $tag passed updater verification."
Write-Host "The release is latest, all updater assets exist, and manifest URLs and hashes match."
