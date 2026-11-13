
use std::fs;
use std::io::{self, Read, Write};
use std::path::Path;
use std::process::Command;
use std::time::Duration;

use reqwest::blocking::Client;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    MB_ICONERROR, MB_ICONINFORMATION, MB_OK, MB_YESNO, MessageBoxW,
};

// The newest desktop release may not be the repository's latest release,
// because backend releases use the same repository but independent versions.
const RELEASE_API_URL: &str =
    "https://api.github.com/repos/Cypherusuh/CypherSR/releases";
const REPOSITORY: &str = "Cypherusuh/CypherSR";

// Release asset naming convention.
const DESKTOP_ASSET_PREFIX: &str = "CypherSR-Desktop-GUI-v";
const DESKTOP_ASSET_SUFFIX_ZIP: &str = "-win-x64.zip";
const DESKTOP_ASSET_SUFFIX_EXE: &str = "-win-x64.exe";

// Installed executable name.
const DESKTOP_EXE_NAME: &str = "CypherSR-Desktop.exe";

const DESKTOP_UPDATE_DIR: &str = ".tmp\\desktop-update";
const STAGED_EXE_NAME: &str = "CypherSR-Desktop.exe";
const DOWNLOADED_ZIP_NAME: &str = "CypherSR-Desktop-GUI.zip";
const PARTIAL_ZIP_NAME: &str = "CypherSR-Desktop-GUI.zip.part";
const PARTIAL_EXE_NAME: &str = "CypherSR-Desktop.exe.part";
const EXTRACT_DIR_NAME: &str = "extracted";
const UPDATE_SCRIPT_NAME: &str = "replace-desktop.ps1";

const HTTP_TIMEOUT_SECS: u64 = 600;

// ------------------------------------------------------------
// GitHub API structures
// ------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct GitHubRelease {
    tag_name: String,
    assets: Vec<GitHubReleaseAsset>,
    prerelease: bool,
}

#[derive(Debug, Deserialize)]
struct GitHubReleaseAsset {
    name: String,

    browser_download_url: String,

    // GitHub provides this as:
    // sha256:0123456789abcdef...
    //
    // Optional in the API schema, but required by our updater.
    digest: Option<String>,

    size: u64,
}

#[derive(Debug)]
struct DesktopUpdateManifest {
    version: String,
    release_tag: String,
    url: String,
    sha256: String,
    size: u64,
    is_zip: bool,
}

// ------------------------------------------------------------
// Public entry point
// ------------------------------------------------------------

/// Returns true when an update has been staged and the app
/// should exit so the external updater can replace the
/// running executable.
pub fn check_and_apply_update() -> bool {
    let current_version = env!("CYPHERSR_APP_VERSION");

    let client = match Client::builder()
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(HTTP_TIMEOUT_SECS))
        .user_agent(format!("CypherSR-Desktop/{current_version}"))
        .build()
    {
        Ok(client) => client,
        Err(error) => {
            let message = format!(
                "Could not check for Desktop updates. \
                 CypherSR will continue with the installed version.\n\n{error}"
            );
            eprintln!("[DesktopUpdate] {message}");
            show_error(&message);
            return false;
        }
    };

    // Fetch latest GitHub release and resolve our desktop asset.
    let manifest = match fetch_latest_desktop_release(&client) {
        Ok(manifest) => manifest,
        Err(error) => {
            let message = format!(
                "Could not check for Desktop updates. \
                 CypherSR will continue with the installed version.\n\n{error}"
            );
            eprintln!("[DesktopUpdate] {message}");
            show_error(&message);
            return false;
        }
    };

    // Compare installed version against the desktop asset version.
    if !is_newer_version(&manifest.version, current_version) {
        println!(
            "[DesktopUpdate] Current version {current_version}; \
             latest desktop version {} (release {}).",
            manifest.version,
            manifest.release_tag
        );

        return false;
    }

    println!(
        "[DesktopUpdate] Update available: {} -> {}",
        current_version,
        manifest.version
    );

    let prompt = format!(
        "A new CypherSR Desktop version is available.\n\n\
         Installed version: {current_version}\n\
         Available version: {}\n\
         GitHub release: {}\n\n\
         Download and install it now?",
        manifest.version,
        manifest.release_tag
    );

    if !ask_yes_no(&prompt) {
        return false;
    }

    match download_and_stage_update(&client, &manifest) {
        Ok(()) => true,

        Err(error) => {
            show_error(&format!(
                "Could not install the desktop update.\n\n\
                 {error}\n\n\
                 The current version will continue running."
            ));

            false
        }
    }
}

// ------------------------------------------------------------
// GitHub release discovery
// ------------------------------------------------------------

fn fetch_latest_desktop_release(
    client: &Client,
) -> Result<DesktopUpdateManifest, Box<dyn std::error::Error>> {
    println!("[DesktopUpdate] Searching stable releases for the latest desktop asset...");

    let mut page = 1_u32;
    let mut all_releases = Vec::new();

    loop {
        let page_query = page.to_string();
        let response = client
            .get(RELEASE_API_URL)
            .query(&[("per_page", "100"), ("page", page_query.as_str())])
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .send()?
            .error_for_status()?;

        let page_releases: Vec<GitHubRelease> = response.json()?;
        if page_releases.is_empty() {
            break;
        }

        all_releases.extend(page_releases);

        page = page
            .checked_add(1)
            .ok_or_else(|| io::Error::other("GitHub release page number overflow"))?;
    }

    select_latest_desktop_release(&all_releases)
}

fn select_latest_desktop_release(
    releases: &[GitHubRelease],
) -> Result<DesktopUpdateManifest, Box<dyn std::error::Error>> {
    let mut latest: Option<DesktopUpdateManifest> = None;
    let mut malformed_assets = Vec::new();

    for release in releases.iter().filter(|release| !release.prerelease) {
        let mut matching_assets: Vec<(&GitHubReleaseAsset, &str, bool)> = Vec::new();

        for asset in &release.assets {
            let Some(name) = asset.name.strip_prefix(DESKTOP_ASSET_PREFIX) else {
                continue;
            };

            if let Some(version) = name.strip_suffix(DESKTOP_ASSET_SUFFIX_ZIP) {
                matching_assets.push((asset, version, true));
            } else if let Some(version) = name.strip_suffix(DESKTOP_ASSET_SUFFIX_EXE) {
                matching_assets.push((asset, version, false));
            } else {
                let lower = name.to_ascii_lowercase();
                if lower.ends_with(".zip") || lower.ends_with(".exe") {
                    if let Some(version) = name
                        .split_once("-win-")
                        .and_then(|(version, _)| parse_version(version))
                    {
                        malformed_assets.push((version, asset.name.as_str()));
                    }
                }
            }
        }

        if matching_assets.is_empty() {
            continue;
        }

        // Prefer ZIP for new/transition releases (extract + replace); keep EXE
        // fallback so already-installed updaters can still update from releases
        // that only provide the EXE asset.
        let (asset, version, is_zip) = matching_assets
            .iter()
            .copied()
            .find(|(_, _, zip)| *zip)
            .unwrap_or(matching_assets[0]);

        let parsed_version = parse_version(version).ok_or_else(|| {
            io::Error::other(format!(
                "Invalid desktop version in release asset: {}",
                asset.name
            ))
        })?;

        let sha256 = asset
            .digest
            .as_deref()
            .and_then(|digest| digest.strip_prefix("sha256:"))
            .ok_or_else(|| {
                io::Error::other(format!(
                    "GitHub did not provide a SHA-256 digest for {}",
                    asset.name
                ))
            })?
            .to_string();

        if sha256.len() != 64 || !sha256.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(io::Error::other(format!(
                "Invalid SHA-256 digest returned for {}",
                asset.name
            ))
            .into());
        }

        let download_url = reqwest::Url::parse(&asset.browser_download_url)?;
        let expected_path = format!(
            "/{REPOSITORY}/releases/download/{}/{}",
            release.tag_name, asset.name
        );
        if download_url.scheme() != "https"
            || download_url.host_str() != Some("github.com")
            || download_url.port().is_some()
            || download_url.username() != ""
            || download_url.password().is_some()
            || download_url.path() != expected_path
            || download_url.query().is_some()
            || download_url.fragment().is_some()
        {
            return Err(io::Error::other(format!(
                "Unexpected download URL for {}: {}",
                asset.name, asset.browser_download_url
            ))
            .into());
        }

        let candidate = DesktopUpdateManifest {
            version: version.to_string(),
            release_tag: release.tag_name.clone(),
            url: asset.browser_download_url.clone(),
            sha256,
            size: asset.size,
            is_zip,
        };

        match latest.as_ref() {
            Some(current) => match parsed_version.cmp(
                &parse_version(&current.version)
                    .ok_or_else(|| io::Error::other("Invalid selected desktop version"))?,
            ) {
                std::cmp::Ordering::Greater => latest = Some(candidate),
                std::cmp::Ordering::Equal | std::cmp::Ordering::Less => {}
            },
            None => latest = Some(candidate),
        }
    }

    if let Some(current) = latest.as_ref() {
        let latest_version = parse_version(&current.version)
            .ok_or_else(|| io::Error::other("Invalid selected desktop version"))?;
        if let Some((_, asset_name)) = malformed_assets
            .iter()
            .filter(|(version, _)| *version >= latest_version)
            .max_by_key(|(version, _)| *version)
        {
            return Err(io::Error::other(format!(
                "A newer Desktop release has a misnamed updater asset: {asset_name}. \
                 Expected {DESKTOP_ASSET_PREFIX}<version>{DESKTOP_ASSET_SUFFIX_ZIP} or \
                 {DESKTOP_ASSET_PREFIX}<version>{DESKTOP_ASSET_SUFFIX_EXE}."
            ))
            .into());
        }
    } else if let Some((_, asset_name)) = malformed_assets.first() {
        return Err(io::Error::other(format!(
            "Desktop updater asset is misnamed: {asset_name}. \
             Expected {DESKTOP_ASSET_PREFIX}<version>{DESKTOP_ASSET_SUFFIX_ZIP} or \
             {DESKTOP_ASSET_PREFIX}<version>{DESKTOP_ASSET_SUFFIX_EXE}."
        ))
        .into());
    }

    latest.ok_or_else(|| {
        io::Error::other(format!(
            "No stable release contains a desktop update asset named \
             {DESKTOP_ASSET_PREFIX}<version>{DESKTOP_ASSET_SUFFIX_ZIP} or \
             {DESKTOP_ASSET_PREFIX}<version>{DESKTOP_ASSET_SUFFIX_EXE}."
        ))
        .into()
    })
}

// ------------------------------------------------------------
// Download and stage update
// ------------------------------------------------------------

fn download_and_stage_update(
    client: &Client,
    manifest: &DesktopUpdateManifest,
) -> Result<(), Box<dyn std::error::Error>> {
    let executable = std::env::current_exe()?;

    let install_dir = executable.parent().ok_or_else(|| {
        io::Error::other(
            "Could not determine the desktop installation directory",
        )
    })?;

    if executable
        .file_name()
        .and_then(|name| name.to_str())
        != Some(DESKTOP_EXE_NAME)
    {
        return Err(io::Error::other(format!(
            "The running executable is not named {DESKTOP_EXE_NAME}.\n\
             Current path: {}",
            executable.display()
        ))
        .into());
    }

    let update_dir = install_dir.join(DESKTOP_UPDATE_DIR);

    fs::create_dir_all(&update_dir)?;

    let staged_executable = update_dir.join(STAGED_EXE_NAME);
    let downloaded_zip = update_dir.join(DOWNLOADED_ZIP_NAME);
    let partial_zip = update_dir.join(PARTIAL_ZIP_NAME);
    let partial_exe = update_dir.join(PARTIAL_EXE_NAME);
    let extract_dir = update_dir.join(EXTRACT_DIR_NAME);
    let script_path = update_dir.join(UPDATE_SCRIPT_NAME);

    // Remove stale files from previous update attempts.
    let _ = fs::remove_file(&staged_executable);
    let _ = fs::remove_file(&downloaded_zip);
    let _ = fs::remove_file(&partial_zip);
    let _ = fs::remove_file(&partial_exe);
    let _ = fs::remove_file(&script_path);
    let _ = fs::remove_dir_all(&extract_dir);

    let partial_path = if manifest.is_zip {
        &partial_zip
    } else {
        &partial_exe
    };

    println!(
        "[DesktopUpdate] Downloading version {} ({}) from {}",
        manifest.version,
        if manifest.is_zip { "zip" } else { "exe" },
        manifest.url
    );

    let mut response = client
        .get(&manifest.url)
        .send()?
        .error_for_status()?;

    // Check Content-Length when the server provides it.
    if let Some(content_length) = response.content_length() {
        if content_length != manifest.size {
            return Err(io::Error::other(format!(
                "Download size mismatch.\n\
                 Expected: {} bytes\n\
                 Received: {} bytes",
                manifest.size,
                content_length
            ))
            .into());
        }
    }

    let mut output = fs::File::create(partial_path)?;
    let mut hasher = Sha256::new();

    let mut buffer = [0_u8; 64 * 1024];
    let mut total = 0_u64;

    loop {
        let count = response.read(&mut buffer)?;

        if count == 0 {
            break;
        }

        total = total
            .checked_add(count as u64)
            .ok_or_else(|| {
                io::Error::other("Desktop update size overflow")
            })?;

        output.write_all(&buffer[..count])?;
        hasher.update(&buffer[..count]);
    }

    output.flush()?;
    output.sync_all()?;
    drop(output);

    if total == 0 {
        let _ = fs::remove_file(partial_path);

        return Err(
            io::Error::other("The downloaded update file is empty").into()
        );
    }

    // Verify downloaded size against GitHub metadata.
    if total != manifest.size {
        let _ = fs::remove_file(partial_path);

        return Err(io::Error::other(format!(
            "Download size verification failed.\n\
             Expected: {} bytes\n\
             Received: {} bytes",
            manifest.size,
            total
        ))
        .into());
    }

    // Verify SHA-256.
    let actual_sha256 = format!("{:x}", hasher.finalize());

    if !actual_sha256.eq_ignore_ascii_case(&manifest.sha256) {
        let _ = fs::remove_file(partial_path);

        return Err(io::Error::other(format!(
            "SHA-256 verification failed.\n\
             Expected: {}\n\
             Received: {}",
            manifest.sha256,
            actual_sha256
        ))
        .into());
    }

    if manifest.is_zip {
        // The download is now verified.
        fs::rename(&partial_zip, &downloaded_zip)?;

        println!(
            "[DesktopUpdate] Verified update archive {}",
            downloaded_zip.display()
        );

        if let Err(error) =
            extract_update_executable(&downloaded_zip, &extract_dir, &staged_executable)
        {
            let _ = fs::remove_file(&downloaded_zip);
            let _ = fs::remove_dir_all(&extract_dir);
            let _ = fs::remove_file(&staged_executable);
            return Err(error.into());
        }

        // Remove the downloaded ZIP after extraction; the replacement script
        // removes the entire update directory (.tmp/desktop-update) on completion.
        let _ = fs::remove_file(&downloaded_zip);
        let _ = fs::remove_dir_all(&extract_dir);

        println!(
            "[DesktopUpdate] Extracted update and staged {}",
            staged_executable.display()
        );
    } else {
        // EXE asset (transition compatibility for already-installed updaters).
        fs::rename(&partial_exe, &staged_executable)?;

        println!(
            "[DesktopUpdate] Verified update and staged {}",
            staged_executable.display()
        );
    }

    if let Err(error) = start_replacement_script(
        &executable,
        &staged_executable,
        std::process::id(),
        &script_path,
    ) {
        let _ = fs::remove_file(&staged_executable);
        let _ = fs::remove_file(&script_path);
        let _ = fs::remove_dir_all(&update_dir);

        return Err(error.into());
    }

    println!(
        "[DesktopUpdate] Handing off replacement to PowerShell."
    );

    Ok(())
}

// ------------------------------------------------------------
// Archive extraction
// ------------------------------------------------------------

fn extract_update_executable(
    zip_path: &Path,
    extract_dir: &Path,
    staged_executable: &Path,
) -> io::Result<()> {
    if extract_dir.exists() {
        fs::remove_dir_all(extract_dir)?;
    }
    fs::create_dir_all(extract_dir)?;

    let status = Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            "Expand-Archive -LiteralPath $args[0] -DestinationPath $args[1] -Force",
        ])
        .arg(zip_path)
        .arg(extract_dir)
        .status()?;

    if !status.success() {
        return Err(io::Error::other(format!(
            "Failed to extract the desktop update archive (exit status: {status})"
        )));
    }

    let extracted = extract_dir.join(DESKTOP_EXE_NAME);
    if !extracted.is_file() {
        return Err(io::Error::other(format!(
            "Desktop update archive did not contain {DESKTOP_EXE_NAME}"
        )));
    }

    if staged_executable.exists() {
        fs::remove_file(staged_executable)?;
    }
    fs::rename(&extracted, staged_executable)?;
    Ok(())
}

// ------------------------------------------------------------
// External executable replacement
// ------------------------------------------------------------

fn start_replacement_script(
    target_executable: &Path,
    new_executable: &Path,
    process_id: u32,
    script_path: &Path,
) -> io::Result<()> {
    let script = r#"
param(
    [Parameter(Mandatory = $true)][int]$ProcessId,
    [Parameter(Mandatory = $true)][string]$NewExe,
    [Parameter(Mandatory = $true)][string]$TargetExe,
    [Parameter(Mandatory = $true)][string]$UpdateDir
)

$ErrorActionPreference = 'Stop'

try {
    # Wait until the current Desktop process exits.
    Wait-Process -Id $ProcessId -ErrorAction SilentlyContinue

    Start-Sleep -Milliseconds 500

    if (-not (Test-Path -LiteralPath $NewExe -PathType Leaf)) {
        throw "Staged executable not found: $NewExe"
    }

    # Replace the old Desktop executable.
    Move-Item `
        -LiteralPath $NewExe `
        -Destination $TargetExe `
        -Force

    # Start the updated Desktop.
    Start-Process `
        -FilePath $TargetExe `
        -WorkingDirectory (Split-Path -Parent $TargetExe)

    # Cleanup temporary update directory.
    Start-Sleep -Milliseconds 500

    Remove-Item `
        -LiteralPath $UpdateDir `
        -Recurse `
        -Force `
        -ErrorAction SilentlyContinue
}
catch {
    $logPath = Join-Path $UpdateDir 'update-error.log'

    try {
        $_ | Out-String | Set-Content -LiteralPath $logPath
    }
    catch {
        # Do not hide the original update failure.
    }

    Add-Type -AssemblyName PresentationFramework

    [System.Windows.MessageBox]::Show(
        "The desktop update could not be completed.`n`n$($_.Exception.Message)`n`nTemporary files were retained in:`n$UpdateDir",
        'CypherSR Desktop Updater',
        'OK',
        'Error'
    ) | Out-Null
}
"#;

    fs::write(script_path, script)?;

    let update_dir = script_path.parent().ok_or_else(|| {
        io::Error::other(
            "Could not determine Desktop update directory"
        )
    })?;

    Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
        ])
        .arg(script_path)
        .arg("-ProcessId")
        .arg(process_id.to_string())
        .arg("-NewExe")
        .arg(new_executable)
        .arg("-TargetExe")
        .arg(target_executable)
        .arg("-UpdateDir")
        .arg(update_dir)
        .spawn()?;

    Ok(())
}

// ------------------------------------------------------------
// Version comparison
// ------------------------------------------------------------

fn parse_version(version: &str) -> Option<(u64, u64, u64)> {
    let version = version.trim().strip_prefix('v').unwrap_or(version.trim());

    let mut parts = version.split('.');

    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts.next()?.parse().ok()?;

    if parts.next().is_some() {
        return None;
    }

    Some((major, minor, patch))
}

fn is_newer_version(candidate: &str, current: &str) -> bool {
    match (parse_version(candidate), parse_version(current)) {
        (Some(candidate), Some(current)) => candidate > current,

        _ => {
            eprintln!(
                "[DesktopUpdate] Invalid version format: \
                 candidate={candidate:?}, current={current:?}"
            );

            false
        }
    }
}

// ------------------------------------------------------------
// Windows dialogs
// ------------------------------------------------------------

fn ask_yes_no(message: &str) -> bool {
    let title = to_wide("CypherSR Desktop");
    let message = to_wide(message);

    unsafe {
        MessageBoxW(
            std::ptr::null_mut(),
            message.as_ptr(),
            title.as_ptr(),
            MB_YESNO | MB_ICONINFORMATION,
        ) == 6 // IDYES
    }
}

fn show_error(message: &str) {
    let title = to_wide("CypherSR Desktop Updater");
    let message = to_wide(message);

    unsafe {
        MessageBoxW(
            std::ptr::null_mut(),
            message.as_ptr(),
            title.as_ptr(),
            MB_OK | MB_ICONERROR,
        );
    }
}

fn to_wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn release(tag: &str, version: &str, prerelease: bool) -> GitHubRelease {
        release_zip(tag, version, prerelease)
    }

    fn release_zip(tag: &str, version: &str, prerelease: bool) -> GitHubRelease {
        let asset_name = format!("{DESKTOP_ASSET_PREFIX}{version}{DESKTOP_ASSET_SUFFIX_ZIP}");

        GitHubRelease {
            tag_name: tag.to_string(),
            prerelease,
            assets: vec![GitHubReleaseAsset {
                browser_download_url: format!(
                    "https://github.com/{REPOSITORY}/releases/download/{tag}/{asset_name}"
                ),
                name: asset_name,
                digest: Some(format!("sha256:{}", "a".repeat(64))),
                size: 1024,
            }],
        }
    }

    fn release_exe(tag: &str, version: &str, prerelease: bool) -> GitHubRelease {
        let asset_name = format!("{DESKTOP_ASSET_PREFIX}{version}{DESKTOP_ASSET_SUFFIX_EXE}");

        GitHubRelease {
            tag_name: tag.to_string(),
            prerelease,
            assets: vec![GitHubReleaseAsset {
                browser_download_url: format!(
                    "https://github.com/{REPOSITORY}/releases/download/{tag}/{asset_name}"
                ),
                name: asset_name,
                digest: Some(format!("sha256:{}", "b".repeat(64))),
                size: 2048,
            }],
        }
    }

    #[test]
    fn selects_highest_desktop_version_independent_of_release_order() {
        let releases = [
            release("v1.0.2", "1.0.2", false),
            release("v1.2.1", "1.2.1", false),
            release("v9.0.0-rc1", "9.0.0", true),
        ];

        let update = select_latest_desktop_release(&releases).unwrap();

        assert_eq!(update.version, "1.2.1");
        assert_eq!(update.release_tag, "v1.2.1");
        assert!(update.is_zip);
    }

    #[test]
    fn accepts_exe_asset_for_transition_compatibility() {
        let releases = [release_exe("v1.2.1", "1.2.1", false)];

        let update = select_latest_desktop_release(&releases).unwrap();

        assert_eq!(update.version, "1.2.1");
        assert!(!update.is_zip);
        assert!(update.url.ends_with(".exe"));
    }

    #[test]
    fn prefers_zip_when_both_assets_are_present() {
        let mut release = release_zip("v1.2.1", "1.2.1", false);
        release.assets.push(release_exe("v1.2.1", "1.2.1", false).assets.remove(0));

        let update = select_latest_desktop_release(&[release]).unwrap();

        assert!(update.is_zip);
        assert!(update.url.ends_with(".zip"));
    }

    #[test]
    fn rejects_release_without_exact_updater_asset_name() {
        let mut release = release("v1.2.1", "1.2.1", false);
        release.assets[0].name = "CypherSR-Desktop-GUI-v1.2.1-win-x6.zip".to_string();

        assert!(select_latest_desktop_release(&[release]).is_err());
    }

    #[test]
    fn reports_newer_misnamed_asset_instead_of_silently_using_old_release() {
        let valid = release("v1.0.2", "1.0.2", false);
        let mut malformed = release("v1.2.1", "1.2.1", false);
        malformed.assets[0].name = "CypherSR-Desktop-GUI-v1.2.1-win-x6.zip".to_string();

        let error = select_latest_desktop_release(&[valid, malformed]).unwrap_err();

        assert!(error.to_string().contains("misnamed updater asset"));
    }

    #[test]
    fn valid_new_release_supersedes_older_misnamed_asset() {
        let mut malformed = release("v1.2.1", "1.2.1", false);
        malformed.assets[0].name = "CypherSR-Desktop-GUI-v1.2.1-win-x6.zip".to_string();
        let current = release("v1.3.0", "1.3.0", false);

        let update = select_latest_desktop_release(&[malformed, current]).unwrap();

        assert_eq!(update.version, "1.3.0");
        assert!(update.is_zip);
    }
}
