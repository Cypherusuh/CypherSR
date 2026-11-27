[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [ValidatePattern('^\d+\.\d+\.\d+$')]
    [string]$FromVersion,

    [Parameter(Mandatory = $true)]
    [ValidatePattern('^\d+\.\d+\.\d+$')]
    [string]$TargetVersion,

    [string]$ReleaseRoot,

    [string]$InstallDir
)

$ErrorActionPreference = "Stop"

$RepoRoot = Split-Path -Parent $PSScriptRoot
$SourceTag = "v$FromVersion"
$Version = [version]::Parse($FromVersion)
$Target = [version]::Parse($TargetVersion)

if ($Target -le $Version) {
    throw "TargetVersion must be newer than FromVersion."
}

if ([string]::IsNullOrWhiteSpace($ReleaseRoot)) {
    $ReleaseRoot = Join-Path $RepoRoot "release"
}
$ReleaseRoot = [System.IO.Path]::GetFullPath($ReleaseRoot)

$ArchiveName = "CypherSR-Desktop-GUI-v$TargetVersion-win-x64.zip"
$ArchivePath = Join-Path $ReleaseRoot "v$TargetVersion\Publishing zip\$ArchiveName"
if (-not (Test-Path -LiteralPath $ArchivePath -PathType Leaf)) {
    throw "Target Desktop ZIP not found: $ArchivePath"
}
if ((Get-Item -LiteralPath $ArchivePath).Length -le 0) {
    throw "Target Desktop ZIP is empty: $ArchivePath"
}

if ([string]::IsNullOrWhiteSpace($InstallDir)) {
    $runId = [guid]::NewGuid().ToString("N")
    $InstallDir = Join-Path $env:TEMP "CypherSR-Updater-Compatibility-Test\$FromVersion-to-$TargetVersion-$runId\install"
}
$InstallDir = [System.IO.Path]::GetFullPath($InstallDir)
$TempRoot = [System.IO.Path]::GetFullPath($env:TEMP).TrimEnd('\', '/')
if (-not $InstallDir.StartsWith("$TempRoot\", [StringComparison]::OrdinalIgnoreCase)) {
    throw "The isolated test install must be inside the system temporary directory: $TempRoot"
}

if (-not (Get-Command "cargo" -ErrorAction SilentlyContinue)) {
    throw "Cargo was not found. Install Rust and reopen the terminal."
}

$SourceDir = Join-Path $env:TEMP ("CypherSR-Updater-Source-" + [guid]::NewGuid().ToString("N"))
$WorktreeAdded = $false
$TestExe = Join-Path $InstallDir "CypherSR-Desktop.exe"
$UpdateDir = Join-Path $InstallDir ".tmp\desktop-update"
$ErrorLog = Join-Path $UpdateDir "legacy-update-error.log"
$ResultLog = Join-Path $UpdateDir "legacy-update-result.log"
$OutputLog = Join-Path $InstallDir "legacy-updater-test.log"
$ErrorOutputLog = Join-Path $InstallDir "legacy-updater-stderr.log"
$EnvironmentNames = @(
    "CYPHERSR_LEGACY_UPDATER_TEST_ARCHIVE",
    "CYPHERSR_LEGACY_UPDATER_TEST_INSTALL_DIR"
)
$PreviousEnvironment = @{}
foreach ($Name in $EnvironmentNames) {
    $PreviousEnvironment[$Name] = [Environment]::GetEnvironmentVariable($Name, "Process")
}

function Replace-SourceTextOnce {
    param(
        [Parameter(Mandatory = $true)][string]$Content,
        [Parameter(Mandatory = $true)][string]$Search,
        [Parameter(Mandatory = $true)][string]$Replacement,
        [Parameter(Mandatory = $true)][string]$Description
    )

    $count = [regex]::Matches($Content, [regex]::Escape($Search)).Count
    if ($count -ne 1) {
        throw "Expected one insertion point for $Description in historical source; found $count."
    }

    return $Content.Replace($Search, $Replacement)
}

try {
    New-Item -ItemType Directory -Path $InstallDir -Force | Out-Null

    Push-Location $RepoRoot
    try {
        git worktree add --detach $SourceDir $SourceTag
        if ($LASTEXITCODE -ne 0) {
            throw "Could not create a temporary worktree for $SourceTag."
        }
        $WorktreeAdded = $true
    }
    finally {
        Pop-Location
    }

    $Utf8NoBom = [System.Text.UTF8Encoding]::new($false)
    $MainPath = Join-Path $SourceDir "desktop\src\main.rs"
    $MainSource = [System.IO.File]::ReadAllText($MainPath)
    $MainAnchor = "fn main() -> Result<(), Box<dyn std::error::Error>> {"
    $MainHook = @'
fn main() -> Result<(), Box<dyn std::error::Error>> {
    if std::env::var_os("CYPHERSR_LEGACY_UPDATER_TEST_ARCHIVE").is_some() {
        return desktop_updater::run_local_legacy_update_test();
    }
'@
    $MainSource = Replace-SourceTextOnce $MainSource $MainAnchor $MainHook "test entry point"
    [System.IO.File]::WriteAllText($MainPath, $MainSource, $Utf8NoBom)

    $UpdaterPath = Join-Path $SourceDir "desktop\src\desktop_updater.rs"
    $UpdaterSource = [System.IO.File]::ReadAllText($UpdaterPath)
    $UpdaterAnchor = "// GitHub release discovery"
    $UpdaterHook = @'
pub fn run_local_legacy_update_test() -> Result<(), Box<dyn std::error::Error>> {
    let archive = std::path::PathBuf::from(
        std::env::var_os("CYPHERSR_LEGACY_UPDATER_TEST_ARCHIVE")
            .ok_or_else(|| io::Error::other("Local test archive path is not set"))?,
    );
    let configured_dir = std::path::PathBuf::from(
        std::env::var_os("CYPHERSR_LEGACY_UPDATER_TEST_INSTALL_DIR")
            .ok_or_else(|| io::Error::other("Local test install directory is not set"))?,
    );
    let executable = std::env::current_exe()?;
    let executable_dir = executable
        .parent()
        .ok_or_else(|| io::Error::other("Could not locate the test executable directory"))?;
    let executable_dir = fs::canonicalize(executable_dir)?;
    let configured_dir = fs::canonicalize(configured_dir)?;
    let temp_dir = fs::canonicalize(std::env::temp_dir())?;

    if executable_dir != configured_dir
        || executable_dir == temp_dir
        || !executable_dir.starts_with(&temp_dir)
    {
        return Err(io::Error::other(
            "Historical updater test executable is not in its isolated temporary directory",
        )
        .into());
    }

    let update_dir = executable_dir.join(DESKTOP_UPDATE_DIR);
    fs::create_dir_all(&update_dir)?;
    let error_log = update_dir.join("legacy-update-error.log");
    let result_log = update_dir.join("legacy-update-result.log");
    if error_log.exists() {
        fs::remove_file(&error_log)?;
    }
    if result_log.exists() {
        fs::remove_file(&result_log)?;
    }

    let local_archive = update_dir.join(DOWNLOADED_ZIP_NAME);
    fs::copy(archive, &local_archive)?;
    let extract_dir = update_dir.join(EXTRACT_DIR_NAME);
    let staged_executable = update_dir.join(STAGED_EXE_NAME);

    if let Err(error) = extract_update_executable(
        &local_archive,
        &extract_dir,
        &staged_executable,
    ) {
        fs::write(&error_log, error.to_string())?;
        return Err(error.into());
    }

    start_replacement_script(
        &executable,
        &staged_executable,
        std::process::id(),
        &update_dir.join(UPDATE_SCRIPT_NAME),
    )?;
    Ok(())
}

// GitHub release discovery
'@
    $UpdaterSource = Replace-SourceTextOnce $UpdaterSource $UpdaterAnchor $UpdaterHook "local archive test entry point"

    $ReplacementAnchor = "# Start the updated Desktop."
    $ReplacementHook = @'
# Start the updated Desktop.
    if ($env:CYPHERSR_LEGACY_UPDATER_TEST_ARCHIVE) {
        if (-not (Test-Path -LiteralPath $TargetExe -PathType Leaf)) {
            throw "Updated test executable not found: $TargetExe"
        }

        "Historical updater replaced the test executable: $TargetExe" |
            Set-Content -LiteralPath (Join-Path $UpdateDir 'legacy-update-result.log')
        exit 0
    }
'@
    $UpdaterSource = Replace-SourceTextOnce $UpdaterSource $ReplacementAnchor $ReplacementHook "test-only no-relaunch guard"
    [System.IO.File]::WriteAllText($UpdaterPath, $UpdaterSource, $Utf8NoBom)

    $CargoManifest = Join-Path $SourceDir "desktop\Cargo.toml"
    Push-Location $SourceDir
    try {
        cargo build --manifest-path $CargoManifest
        if ($LASTEXITCODE -ne 0) {
            throw "Historical updater build failed with exit code $LASTEXITCODE."
        }
    }
    finally {
        Pop-Location
    }

    $BuiltExe = Join-Path $SourceDir "desktop\target\debug\CypherSR-Desktop.exe"
    if (-not (Test-Path -LiteralPath $BuiltExe -PathType Leaf)) {
        throw "Historical updater executable not found: $BuiltExe"
    }
    Copy-Item -LiteralPath $BuiltExe -Destination $TestExe -Force

    [Environment]::SetEnvironmentVariable(
        "CYPHERSR_LEGACY_UPDATER_TEST_ARCHIVE",
        [System.IO.Path]::GetFullPath($ArchivePath),
        "Process"
    )
    [Environment]::SetEnvironmentVariable(
        "CYPHERSR_LEGACY_UPDATER_TEST_INSTALL_DIR",
        $InstallDir,
        "Process"
    )

    Write-Host "Bootstrap updater source: $SourceTag"
    Write-Host "Local target package:     $ArchivePath"
    Write-Host "Isolated test install:    $InstallDir"
    Write-Host "Extraction/replacement log: $OutputLog"

    $TestProcess = Start-Process `
        -FilePath $TestExe `
        -WorkingDirectory $InstallDir `
        -Wait `
        -PassThru `
        -WindowStyle Hidden `
        -RedirectStandardOutput $OutputLog `
        -RedirectStandardError $ErrorOutputLog
    $TestExitCode = $TestProcess.ExitCode

    if (Test-Path -LiteralPath $ErrorLog -PathType Leaf) {
        Write-Host "Historical updater failed while extracting the local ZIP:"
        Get-Content -LiteralPath $ErrorLog
        if (Test-Path -LiteralPath $ErrorOutputLog -PathType Leaf) {
            Get-Content -LiteralPath $ErrorOutputLog
        }
        throw "The $SourceTag updater could not install target version $TargetVersion."
    }
    elseif ($TestExitCode -ne 0) {
        Write-Host "Historical updater process output:"
        if (Test-Path -LiteralPath $OutputLog -PathType Leaf) {
            Get-Content -LiteralPath $OutputLog
        }
        if (Test-Path -LiteralPath $ErrorOutputLog -PathType Leaf) {
            Get-Content -LiteralPath $ErrorOutputLog
        }
        throw "The $SourceTag updater test exited with code $TestExitCode; target version $TargetVersion was not installed."
    }
    else {
        $deadline = (Get-Date).AddSeconds(30)
        while (-not (Test-Path -LiteralPath $ResultLog -PathType Leaf) -and (Get-Date) -lt $deadline) {
            Start-Sleep -Milliseconds 200
        }

        if (-not (Test-Path -LiteralPath $ResultLog -PathType Leaf)) {
            throw "Replacement did not report a result. See $OutputLog"
        }

        Write-Host "Historical updater replaced the test copy successfully:"
        Get-Content -LiteralPath $ResultLog
    }
}
finally {
    foreach ($Name in $EnvironmentNames) {
        [Environment]::SetEnvironmentVariable($Name, $PreviousEnvironment[$Name], "Process")
    }

    if ($WorktreeAdded) {
        Push-Location $RepoRoot
        try {
            git worktree remove --force $SourceDir
            if ($LASTEXITCODE -ne 0) {
                Write-Warning "Could not remove temporary historical source worktree: $SourceDir"
            }
        }
        finally {
            Pop-Location
        }
    }
}
