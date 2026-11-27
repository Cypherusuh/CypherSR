use std::fs;
#[cfg(any(not(feature = "local-updater-test"), test))]
use std::io::Read;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
#[cfg(not(feature = "local-updater-test"))]
use std::time::Duration;

#[cfg(any(not(feature = "local-updater-test"), test))]
use reqwest::blocking::Client;
#[cfg(any(not(feature = "local-updater-test"), test))]
use serde::Deserialize;
#[cfg(any(not(feature = "local-updater-test"), test))]
use sha2::{Digest, Sha256};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    MB_ICONERROR, MB_ICONINFORMATION, MB_OK, MB_YESNO, MessageBoxW,
};

// The newest desktop release may not be the repository's latest release,
// because backend releases use the same repository but independent versions.
#[cfg(not(feature = "local-updater-test"))]
const RELEASE_API_URL: &str = "https://api.github.com/repos/Cypherusuh/CypherSR/releases";
#[cfg(any(not(feature = "local-updater-test"), test))]
const REPOSITORY: &str = "Cypherusuh/CypherSR";

// Release asset naming convention.
const DESKTOP_ASSET_PREFIX: &str = "CypherSR-Desktop-GUI-v";
const DESKTOP_ASSET_SUFFIX_ZIP: &str = "-win-x64.zip";
#[cfg(any(not(feature = "local-updater-test"), test))]
const DESKTOP_ASSET_SUFFIX_EXE: &str = "-win-x64.exe";

// Installed executable name.
const DESKTOP_EXE_NAME: &str = "CypherSR-Desktop.exe";

#[cfg(feature = "local-updater-test")]
const LOCAL_PUBLISHING_ZIP_DIR: &str = "Publishing zip";
#[cfg(feature = "local-updater-test")]
const LOCAL_TEST_VERSION_ENV: &str = "CYPHERSR_LOCAL_UPDATE_TEST_CURRENT_VERSION";
#[cfg(feature = "local-updater-test")]
const LOCAL_TEST_RELEASE_DIR_ENV: &str = "CYPHERSR_LOCAL_UPDATE_TEST_RELEASE_DIR";
#[cfg(feature = "local-updater-test")]
const LOCAL_TEST_INSTALL_DIR_ENV: &str = "CYPHERSR_LOCAL_UPDATE_TEST_INSTALL_DIR";

const DESKTOP_UPDATE_DIR: &str = ".tmp\\desktop-update";
const STAGED_EXE_NAME: &str = "CypherSR-Desktop.exe";
const DOWNLOADED_ZIP_NAME: &str = "CypherSR-Desktop-GUI.zip";
const PARTIAL_ZIP_NAME: &str = "CypherSR-Desktop-GUI.zip.part";
const PARTIAL_EXE_NAME: &str = "CypherSR-Desktop.exe.part";
const EXTRACT_DIR_NAME: &str = "extracted";
const EXTRACT_SCRIPT_NAME: &str = "extract-desktop-update.ps1";
const UPDATE_SCRIPT_NAME: &str = "replace-desktop.ps1";
const UPDATE_RESULT_LOG_NAME: &str = "update-result.log";
const UPDATE_ERROR_LOG_NAME: &str = "update-error.log";

#[cfg(not(feature = "local-updater-test"))]
const HTTP_TIMEOUT_SECS: u64 = 600;

// Metadata requests (GitHub release listing) must fail fast so a stalled
// connection cannot freeze startup. The long timeout above is only for the
// asset download.
#[cfg(not(feature = "local-updater-test"))]
const API_TIMEOUT_SECS: u64 = 20;

const EXTRACT_SCRIPT: &str = r#"
param(
    [Parameter(Mandatory = $true)][string]$ArchivePath,
    [Parameter(Mandatory = $true)][string]$DestinationPath
)

$ErrorActionPreference = 'Stop'

Expand-Archive `
    -LiteralPath $ArchivePath `
    -DestinationPath $DestinationPath `
    -Force `
    -ErrorAction Stop
"#;

// ------------------------------------------------------------
// GitHub API structures
// ------------------------------------------------------------

#[cfg(any(not(feature = "local-updater-test"), test))]
#[derive(Debug, Deserialize)]
struct GitHubRelease {
    tag_name: String,
    assets: Vec<GitHubReleaseAsset>,
    prerelease: bool,
}

#[cfg(any(not(feature = "local-updater-test"), test))]
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

#[cfg(any(not(feature = "local-updater-test"), test))]
#[derive(Debug)]
struct DesktopUpdateManifest {
    version: String,
    release_tag: String,
    primary: DesktopUpdateAsset,
    fallback_exe: Option<DesktopUpdateAsset>,
}

#[cfg(any(not(feature = "local-updater-test"), test))]
#[derive(Debug)]
struct DesktopUpdateAsset {
    url: String,
    sha256: String,
    size: u64,
    is_zip: bool,
}

#[cfg(feature = "local-updater-test")]
#[derive(Debug)]
struct LocalDesktopUpdate {
    version: String,
    archive: PathBuf,
}

struct UpdateStagingPaths {
    executable: PathBuf,
    update_dir: PathBuf,
    staged_executable: PathBuf,
    script_path: PathBuf,
}

// ------------------------------------------------------------
// Public entry point
// ------------------------------------------------------------

/// Returns true when an update has been staged and the app
/// should exit so the external updater can replace the
/// running executable.
#[cfg(not(feature = "local-updater-test"))]
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
            manifest.version, manifest.release_tag
        );

        return false;
    }

    println!(
        "[DesktopUpdate] Update available: {} -> {}",
        current_version, manifest.version
    );

    let prompt = format!(
        "A new CypherSR Desktop version is available.\n\n\
         Installed version: {current_version}\n\
         Available version: {}\n\
         GitHub release: {}\n\n\
         Download and install it now?",
        manifest.version, manifest.release_tag
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

#[cfg(feature = "local-updater-test")]
pub fn check_and_apply_update() -> bool {
    if let Err(error) = validate_local_test_install() {
        show_error(&format!(
            "Local updater tests must run from their isolated temporary install directory.\n\n{error}"
        ));
        return false;
    }

    let current_version = match std::env::var(LOCAL_TEST_VERSION_ENV) {
        Ok(version) => version,
        Err(std::env::VarError::NotPresent) => env!("CYPHERSR_APP_VERSION").to_string(),
        Err(error) => {
            show_error(&format!(
                "Could not read the local updater test version from {LOCAL_TEST_VERSION_ENV}.\n\n{error}"
            ));
            return false;
        }
    };

    match find_newer_local_desktop_update(&current_version) {
        Ok(Some(update)) => {
            println!(
                "[DesktopUpdate] Found local update {} at {}",
                update.version,
                update.archive.display()
            );

            let prompt = format!(
                "A local Desktop update is available.\n\n\
                 Test current version: {current_version}\n\
                 Available version: {}\n\
                 Publishing ZIP: {}\n\n\
                 Install it in this test copy?",
                update.version,
                update.archive.display()
            );

            if !ask_yes_no(&prompt) {
                return false;
            }

            match stage_local_update(&update) {
                Ok(()) => true,
                Err(error) => {
                    show_error(&format!(
                        "Could not install the local test update.\n\n\
                         {error}\n\n\
                         The current version will continue running."
                    ));
                    false
                }
            }
        }
        Ok(None) => {
            println!(
                "[DesktopUpdate] No newer local Publishing zip found; GitHub checks are disabled in local updater test mode."
            );
            false
        }
        Err(error) => {
            show_error(&format!(
                "Could not inspect local Desktop Publishing zips.\n\n{error}"
            ));
            false
        }
    }
}

// ------------------------------------------------------------
// Local release discovery
// ------------------------------------------------------------

#[cfg(feature = "local-updater-test")]
fn find_newer_local_desktop_update(
    current_version: &str,
) -> Result<Option<LocalDesktopUpdate>, Box<dyn std::error::Error>> {
    find_newer_local_update_in_roots(&discover_local_release_roots()?, current_version)
}

#[cfg(feature = "local-updater-test")]
fn find_newer_local_update_in_roots(
    release_roots: &[PathBuf],
    current_version: &str,
) -> Result<Option<LocalDesktopUpdate>, Box<dyn std::error::Error>> {
    let current_version = parse_version(current_version)
        .ok_or_else(|| io::Error::other("Invalid installed Desktop version"))?;
    let mut latest: Option<((u64, u64, u64), LocalDesktopUpdate)> = None;

    for release_root in release_roots {
        let entries = match fs::read_dir(release_root) {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error.into()),
        };

        for entry in entries {
            let entry = entry?;
            if !entry.file_type()?.is_dir() {
                continue;
            }

            let version_name = entry.file_name().to_string_lossy().into_owned();
            let Some(parsed_version) = parse_version(&version_name) else {
                continue;
            };
            if parsed_version <= current_version {
                continue;
            }

            let version = version_name
                .strip_prefix('v')
                .unwrap_or(&version_name)
                .to_string();
            let Some(archive) = find_local_desktop_archive(&entry.path(), &version)? else {
                eprintln!(
                    "[DesktopUpdate] Skipping local release {version_name}: \
                     its Publishing zip was not found or was empty."
                );
                continue;
            };

            let candidate = LocalDesktopUpdate { version, archive };

            if latest
                .as_ref()
                .is_none_or(|(latest_version, _)| parsed_version > *latest_version)
            {
                latest = Some((parsed_version, candidate));
            }
        }
    }

    Ok(latest.map(|(_, update)| update))
}

#[cfg(feature = "local-updater-test")]
fn discover_local_release_roots() -> Result<Vec<PathBuf>, Box<dyn std::error::Error>> {
    let mut roots = Vec::new();

    if let Some(release_root) = std::env::var_os(LOCAL_TEST_RELEASE_DIR_ENV) {
        let release_root = PathBuf::from(release_root);
        if !release_root.is_dir() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!(
                    "Local updater test release directory does not exist: {}",
                    release_root.display()
                ),
            )
            .into());
        }
        push_unique_path(&mut roots, release_root);
        return Ok(roots);
    }

    match std::env::current_dir() {
        Ok(current_dir) => {
            push_unique_path(&mut roots, current_dir.join("release"));
            if current_dir
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.eq_ignore_ascii_case("release"))
            {
                push_unique_path(&mut roots, current_dir);
            }
        }
        Err(error) => {
            eprintln!("[DesktopUpdate] Could not inspect the current directory: {error}");
        }
    }

    match std::env::current_exe() {
        Ok(executable) => {
            for ancestor in executable.ancestors() {
                if ancestor
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.eq_ignore_ascii_case("release"))
                {
                    push_unique_path(&mut roots, ancestor.to_path_buf());
                }
            }
        }
        Err(error) => {
            eprintln!("[DesktopUpdate] Could not locate the running executable: {error}");
        }
    }

    Ok(roots)
}

#[cfg(feature = "local-updater-test")]
fn push_unique_path(paths: &mut Vec<PathBuf>, path: PathBuf) {
    if !paths.contains(&path) {
        paths.push(path);
    }
}

#[cfg(feature = "local-updater-test")]
fn validate_local_test_install() -> io::Result<()> {
    let configured_install_dir = std::env::var_os(LOCAL_TEST_INSTALL_DIR_ENV)
        .ok_or_else(|| io::Error::other(format!("{LOCAL_TEST_INSTALL_DIR_ENV} is not set")))?;
    let configured_install_dir = fs::canonicalize(PathBuf::from(configured_install_dir))?;
    let executable = std::env::current_exe()?;
    let executable_dir = executable
        .parent()
        .ok_or_else(|| io::Error::other("Could not determine the test executable directory"))?;
    let executable_dir = fs::canonicalize(executable_dir)?;
    let temporary_dir = fs::canonicalize(std::env::temp_dir())?;

    validate_local_test_install_paths(&executable_dir, &configured_install_dir, &temporary_dir)
}

#[cfg(feature = "local-updater-test")]
fn validate_local_test_install_paths(
    executable_dir: &Path,
    configured_install_dir: &Path,
    temporary_dir: &Path,
) -> io::Result<()> {
    if !executable_dir.starts_with(temporary_dir) || executable_dir == temporary_dir {
        return Err(io::Error::other(format!(
            "The test executable must be inside the system temporary directory ({}).",
            temporary_dir.display(),
        )));
    }

    if !executable_dir
        .to_string_lossy()
        .eq_ignore_ascii_case(&configured_install_dir.to_string_lossy())
    {
        return Err(io::Error::other(format!(
            "The test executable is not in the configured isolated install directory.\n\
             Executable directory: {}\n\
             Configured directory: {}",
            executable_dir.display(),
            configured_install_dir.display()
        )));
    }

    Ok(())
}

#[cfg(feature = "local-updater-test")]
fn find_local_desktop_archive(
    release_directory: &Path,
    version: &str,
) -> io::Result<Option<PathBuf>> {
    let archive_name = format!("{DESKTOP_ASSET_PREFIX}{version}{DESKTOP_ASSET_SUFFIX_ZIP}");
    let archive = release_directory
        .join(LOCAL_PUBLISHING_ZIP_DIR)
        .join(archive_name);

    match fs::metadata(&archive) {
        Ok(metadata) if metadata.is_file() && metadata.len() > 0 => Ok(Some(archive)),
        Ok(_) => Ok(None),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

// ------------------------------------------------------------
// GitHub release discovery
// ------------------------------------------------------------

#[cfg(not(feature = "local-updater-test"))]
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
            .timeout(Duration::from_secs(API_TIMEOUT_SECS))
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

#[cfg(any(not(feature = "local-updater-test"), test))]
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

        // Prefer the smaller ZIP, but retain an EXE fallback for failed extraction
        // and for transition releases that only provide the executable.
        let (asset, version, is_zip) = matching_assets
            .iter()
            .copied()
            .find(|(_, _, zip)| *zip)
            .unwrap_or(matching_assets[0]);

        let Some(parsed_version) = parse_version(version) else {
            eprintln!(
                "[DesktopUpdate] Skipping release {}: invalid desktop version in asset {}.",
                release.tag_name, asset.name
            );
            continue;
        };

        // Prefer a usable EXE from this release if its ZIP cannot be verified.
        // Otherwise skip the release so one bad asset does not abort the check.
        let mut selected_is_zip = is_zip;
        let primary = match create_desktop_update_asset(&release.tag_name, asset, is_zip) {
            Ok(primary) => primary,
            Err(zip_error) if is_zip => {
                let fallback = matching_assets.iter().copied().find(
                    |(_, candidate_version, candidate_is_zip)| {
                        !candidate_is_zip && *candidate_version == version
                    },
                );

                let Some((fallback, _, _)) = fallback else {
                    eprintln!(
                        "[DesktopUpdate] Skipping release {}: unusable desktop asset {}: {zip_error}",
                        release.tag_name, asset.name
                    );
                    continue;
                };

                match create_desktop_update_asset(&release.tag_name, fallback, false) {
                    Ok(primary) => {
                        eprintln!(
                            "[DesktopUpdate] ZIP asset {} is unusable: {zip_error}. Using EXE fallback {}.",
                            asset.name, fallback.name
                        );
                        selected_is_zip = false;
                        primary
                    }
                    Err(exe_error) => {
                        eprintln!(
                            "[DesktopUpdate] Skipping release {}: ZIP asset {} is unusable: {zip_error}; EXE fallback {} is unusable: {exe_error}",
                            release.tag_name, asset.name, fallback.name
                        );
                        continue;
                    }
                }
            }
            Err(error) => {
                eprintln!(
                    "[DesktopUpdate] Skipping release {}: unusable desktop asset {}: {error}",
                    release.tag_name, asset.name
                );
                continue;
            }
        };

        let fallback_exe =
            if selected_is_zip {
                if let Some((fallback, _, fallback_is_zip)) = matching_assets.iter().copied().find(
                    |(_, candidate_version, candidate_is_zip)| {
                        !candidate_is_zip && *candidate_version == version
                    },
                ) {
                    match create_desktop_update_asset(&release.tag_name, fallback, fallback_is_zip)
                    {
                        Ok(asset) => Some(asset),
                        Err(error) => {
                            eprintln!(
                                "[DesktopUpdate] Ignoring unusable EXE fallback {}: {error}",
                                fallback.name
                            );
                            None
                        }
                    }
                } else {
                    None
                }
            } else {
                None
            };

        let candidate = DesktopUpdateManifest {
            version: version.to_string(),
            release_tag: release.tag_name.clone(),
            primary,
            fallback_exe,
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
            .filter(|(version, _)| *version > latest_version)
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

#[cfg(any(not(feature = "local-updater-test"), test))]
fn create_desktop_update_asset(
    release_tag: &str,
    asset: &GitHubReleaseAsset,
    is_zip: bool,
) -> Result<DesktopUpdateAsset, Box<dyn std::error::Error>> {
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
        "/{REPOSITORY}/releases/download/{release_tag}/{}",
        asset.name
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

    Ok(DesktopUpdateAsset {
        url: asset.browser_download_url.clone(),
        sha256,
        size: asset.size,
        is_zip,
    })
}

// ------------------------------------------------------------
// Download and stage update
// ------------------------------------------------------------

#[cfg(not(feature = "local-updater-test"))]
fn download_and_stage_update(
    client: &Client,
    manifest: &DesktopUpdateManifest,
) -> Result<(), Box<dyn std::error::Error>> {
    let paths = prepare_update_paths()?;
    stage_update_assets(client, manifest, &paths.update_dir)?;
    hand_off_staged_update(&paths)
}

#[cfg(feature = "local-updater-test")]
fn stage_local_update(update: &LocalDesktopUpdate) -> Result<(), Box<dyn std::error::Error>> {
    let paths = prepare_update_paths()?;
    let local_archive = paths.update_dir.join(DOWNLOADED_ZIP_NAME);
    let extract_dir = paths.update_dir.join(EXTRACT_DIR_NAME);
    let extract_script = paths.update_dir.join(EXTRACT_SCRIPT_NAME);
    let stage_result: Result<(), Box<dyn std::error::Error>> = (|| {
        copy_local_archive(&update.archive, &local_archive)?;
        extract_update_executable(
            &local_archive,
            &extract_dir,
            &paths.staged_executable,
            &extract_script,
        )?;
        Ok(())
    })();

    if let Err(error) = stage_result {
        let _ = fs::remove_file(&local_archive);
        let _ = fs::remove_file(&paths.staged_executable);
        let _ = fs::remove_file(&extract_script);
        let _ = fs::remove_dir_all(&extract_dir);
        let _ = fs::remove_dir_all(&paths.update_dir);
        return Err(error);
    }

    let _ = fs::remove_file(local_archive);
    let _ = fs::remove_dir_all(extract_dir);
    hand_off_staged_update(&paths)
}

fn prepare_update_paths() -> Result<UpdateStagingPaths, Box<dyn std::error::Error>> {
    let executable = std::env::current_exe()?;
    let install_dir = executable.parent().ok_or_else(|| {
        io::Error::other("Could not determine the desktop installation directory")
    })?;

    if executable.file_name().and_then(|name| name.to_str()) != Some(DESKTOP_EXE_NAME) {
        return Err(io::Error::other(format!(
            "The running executable is not named {DESKTOP_EXE_NAME}.\n\
             Current path: {}",
            executable.display()
        ))
        .into());
    }

    let update_dir = install_dir.join(DESKTOP_UPDATE_DIR);
    fs::create_dir_all(&update_dir)?;
    clear_stale_update_files(&update_dir);

    Ok(UpdateStagingPaths {
        executable,
        staged_executable: update_dir.join(STAGED_EXE_NAME),
        script_path: update_dir.join(UPDATE_SCRIPT_NAME),
        update_dir,
    })
}

fn clear_stale_update_files(update_dir: &Path) {
    let _ = fs::remove_file(update_dir.join(STAGED_EXE_NAME));
    let _ = fs::remove_file(update_dir.join(DOWNLOADED_ZIP_NAME));
    let _ = fs::remove_file(update_dir.join(PARTIAL_ZIP_NAME));
    let _ = fs::remove_file(update_dir.join(PARTIAL_EXE_NAME));
    let _ = fs::remove_file(update_dir.join(EXTRACT_SCRIPT_NAME));
    let _ = fs::remove_file(update_dir.join(UPDATE_SCRIPT_NAME));
    let _ = fs::remove_file(update_dir.join(UPDATE_RESULT_LOG_NAME));
    let _ = fs::remove_file(update_dir.join(UPDATE_ERROR_LOG_NAME));
    let _ = fs::remove_dir_all(update_dir.join(EXTRACT_DIR_NAME));
}

#[cfg(feature = "local-updater-test")]
fn copy_local_archive(source: &Path, destination: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let mut source = fs::File::open(source)?;
    let expected_size = source.metadata()?.len();
    if expected_size == 0 {
        return Err(io::Error::other("The local Desktop ZIP is empty").into());
    }

    let mut destination_file = fs::File::create(destination)?;
    let copied_size = io::copy(&mut source, &mut destination_file)?;
    destination_file.flush()?;
    destination_file.sync_all()?;

    if copied_size != expected_size {
        return Err(io::Error::other(format!(
            "Local ZIP copy size mismatch. Expected {expected_size} bytes, copied {copied_size} bytes."
        ))
        .into());
    }

    Ok(())
}

fn hand_off_staged_update(paths: &UpdateStagingPaths) -> Result<(), Box<dyn std::error::Error>> {
    if let Err(error) = start_replacement_script(
        &paths.executable,
        &paths.staged_executable,
        std::process::id(),
        &paths.script_path,
    ) {
        let _ = fs::remove_file(&paths.staged_executable);
        let _ = fs::remove_file(&paths.script_path);
        let _ = fs::remove_dir_all(&paths.update_dir);
        return Err(error.into());
    }

    println!("[DesktopUpdate] Handing off replacement to PowerShell.");
    Ok(())
}

#[cfg(any(not(feature = "local-updater-test"), test))]
fn stage_update_assets(
    client: &Client,
    manifest: &DesktopUpdateManifest,
    update_dir: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    let staged_executable = update_dir.join(STAGED_EXE_NAME);

    if let Err(zip_error) = download_and_stage_asset(
        client,
        &manifest.version,
        &manifest.primary,
        update_dir,
        &staged_executable,
    ) {
        if !manifest.primary.is_zip {
            return Err(zip_error);
        }

        let Some(fallback_exe) = &manifest.fallback_exe else {
            return Err(zip_error);
        };

        eprintln!(
            "[DesktopUpdate] ZIP update failed: {zip_error}. Trying the direct EXE fallback."
        );

        if let Err(exe_error) = download_and_stage_asset(
            client,
            &manifest.version,
            fallback_exe,
            update_dir,
            &staged_executable,
        ) {
            return Err(io::Error::other(format!(
                "ZIP update failed: {zip_error}\n\
                 Direct EXE fallback failed: {exe_error}"
            ))
            .into());
        }
    }

    Ok(())
}

#[cfg(any(not(feature = "local-updater-test"), test))]
fn download_and_stage_asset(
    client: &Client,
    version: &str,
    asset: &DesktopUpdateAsset,
    update_dir: &Path,
    staged_executable: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    let downloaded_zip = update_dir.join(DOWNLOADED_ZIP_NAME);
    let partial_path = update_dir.join(if asset.is_zip {
        PARTIAL_ZIP_NAME
    } else {
        PARTIAL_EXE_NAME
    });
    let extract_dir = update_dir.join(EXTRACT_DIR_NAME);
    let extract_script_path = update_dir.join(EXTRACT_SCRIPT_NAME);

    let result: Result<(), Box<dyn std::error::Error>> = (|| {
        println!(
            "[DesktopUpdate] Downloading version {version} ({}) from {}",
            if asset.is_zip { "zip" } else { "exe" },
            asset.url
        );

        let mut response = client.get(&asset.url).send()?.error_for_status()?;

        if let Some(content_length) = response.content_length() {
            if content_length != asset.size {
                return Err(io::Error::other(format!(
                    "Download size mismatch.\n\
                     Expected: {} bytes\n\
                     Received: {} bytes",
                    asset.size, content_length
                ))
                .into());
            }
        }

        let mut output = fs::File::create(&partial_path)?;
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
                .ok_or_else(|| io::Error::other("Desktop update size overflow"))?;

            if total > asset.size {
                return Err(io::Error::other(format!(
                    "Download exceeded the expected size of {} bytes",
                    asset.size
                ))
                .into());
            }

            output.write_all(&buffer[..count])?;
            hasher.update(&buffer[..count]);
        }

        output.flush()?;
        output.sync_all()?;
        drop(output);

        if total == 0 {
            return Err(io::Error::other("The downloaded update file is empty").into());
        }

        if total != asset.size {
            return Err(io::Error::other(format!(
                "Download size verification failed.\n\
                 Expected: {} bytes\n\
                 Received: {} bytes",
                asset.size, total
            ))
            .into());
        }

        let actual_sha256 = format!("{:x}", hasher.finalize());

        if !actual_sha256.eq_ignore_ascii_case(&asset.sha256) {
            return Err(io::Error::other(format!(
                "SHA-256 verification failed.\n\
                 Expected: {}\n\
                 Received: {}",
                asset.sha256, actual_sha256
            ))
            .into());
        }

        if asset.is_zip {
            fs::rename(&partial_path, &downloaded_zip)?;

            println!(
                "[DesktopUpdate] Verified update archive {}",
                downloaded_zip.display()
            );

            extract_update_executable(
                &downloaded_zip,
                &extract_dir,
                staged_executable,
                &extract_script_path,
            )?;

            let _ = fs::remove_file(&downloaded_zip);
            let _ = fs::remove_dir_all(&extract_dir);

            println!(
                "[DesktopUpdate] Extracted update and staged {}",
                staged_executable.display()
            );
        } else {
            fs::rename(&partial_path, staged_executable)?;

            println!(
                "[DesktopUpdate] Verified update and staged {}",
                staged_executable.display()
            );
        }

        Ok(())
    })();

    if result.is_err() {
        let _ = fs::remove_file(&partial_path);
        let _ = fs::remove_file(&downloaded_zip);
        let _ = fs::remove_dir_all(&extract_dir);
        let _ = fs::remove_file(staged_executable);
    }

    result
}

// ------------------------------------------------------------
// Archive extraction
// ------------------------------------------------------------

fn extract_update_executable(
    zip_path: &Path,
    extract_dir: &Path,
    staged_executable: &Path,
    script_path: &Path,
) -> io::Result<()> {
    if extract_dir.exists() {
        fs::remove_dir_all(extract_dir)?;
    }
    fs::create_dir_all(extract_dir)?;
    fs::write(script_path, EXTRACT_SCRIPT)?;

    let output = Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
        ])
        .arg(script_path)
        .arg(zip_path)
        .arg(extract_dir)
        .output();

    if let Err(error) = fs::remove_file(script_path) {
        eprintln!(
            "[DesktopUpdate] Could not remove extraction script {}: {error}",
            script_path.display()
        );
    }

    let output = output?;

    if !output.status.success() {
        let details = [
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        ]
        .into_iter()
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty())
        .collect::<Vec<_>>()
        .join("\n");

        let detail = if details.is_empty() {
            String::new()
        } else {
            format!("\n{details}")
        };

        return Err(io::Error::other(format!(
            "Failed to extract the desktop update archive (exit status: {}){detail}",
            output.status
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

    __LOCAL_TEST_REPLACEMENT_RESULT__

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

    fs::write(
        script_path,
        script.replace(
            "__LOCAL_TEST_REPLACEMENT_RESULT__",
            local_test_replacement_result(),
        ),
    )?;

    let update_dir = script_path
        .parent()
        .ok_or_else(|| io::Error::other("Could not determine Desktop update directory"))?;

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

fn local_test_replacement_result() -> &'static str {
    if cfg!(feature = "local-updater-test") {
        r#"
    if (-not (Test-Path -LiteralPath $TargetExe -PathType Leaf)) {
        throw "Updated test executable not found: $TargetExe"
    }

    $updatedLength = (Get-Item -LiteralPath $TargetExe).Length
    if ($updatedLength -le 0) {
        throw "Updated test executable is empty: $TargetExe"
    }

    $resultPath = Join-Path $UpdateDir 'update-result.log'
    "Local updater test replaced the executable ($updatedLength bytes): $TargetExe" |
        Set-Content -LiteralPath $resultPath
    exit 0
"#
    } else {
        ""
    }
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

#[cfg(not(feature = "local-updater-test"))]
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

    #[cfg(feature = "local-updater-test")]
    #[test]
    fn local_test_replacement_script_records_success_without_relaunching_release_exe() {
        let result_action = local_test_replacement_result();

        assert!(result_action.contains("update-result.log"));
        assert!(result_action.contains("exit 0"));
        assert!(!result_action.contains("Start-Process"));
    }

    #[cfg(feature = "local-updater-test")]
    #[test]
    fn local_test_executable_must_match_install_dir_inside_temp() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock should be after Unix epoch")
            .as_nanos();
        let temporary_dir = std::env::temp_dir().join(format!(
            "cyphersr-local-updater-path-{}-{unique}",
            std::process::id()
        ));
        let install_dir = temporary_dir.join("install");
        let other_install_dir = temporary_dir.join("other-install");

        fs::create_dir_all(&install_dir).expect("test install directory should be created");
        fs::create_dir_all(&other_install_dir).expect("other install directory should be created");

        let temporary_dir =
            fs::canonicalize(&temporary_dir).expect("temporary root should canonicalize");
        let install_dir =
            fs::canonicalize(&install_dir).expect("install directory should canonicalize");
        let other_install_dir =
            fs::canonicalize(&other_install_dir).expect("other directory should canonicalize");

        validate_local_test_install_paths(&install_dir, &install_dir, &temporary_dir)
            .expect("matching test install inside temp should be allowed");
        assert!(
            validate_local_test_install_paths(&install_dir, &other_install_dir, &temporary_dir)
                .is_err(),
            "a different configured directory should be rejected"
        );
        assert!(
            validate_local_test_install_paths(
                &std::env::current_dir().expect("working directory should be readable"),
                &install_dir,
                &temporary_dir
            )
            .is_err(),
            "an executable outside temp should be rejected"
        );

        fs::remove_dir_all(temporary_dir).expect("temporary test paths should be removed");
    }

    #[cfg(feature = "local-updater-test")]
    #[test]
    fn finds_newest_local_publishing_zip_newer_than_installed() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock should be after Unix epoch")
            .as_nanos();
        let temporary_root = std::env::temp_dir().join(format!(
            "cyphersr-local-updater-{}-{unique}",
            std::process::id()
        ));
        let release_root = temporary_root.join("release");
        let current_archive = release_root
            .join("v1.2.3")
            .join(LOCAL_PUBLISHING_ZIP_DIR)
            .join(format!(
                "{DESKTOP_ASSET_PREFIX}1.2.3{DESKTOP_ASSET_SUFFIX_ZIP}"
            ));
        let newer_archive = release_root
            .join("v1.2.4")
            .join(LOCAL_PUBLISHING_ZIP_DIR)
            .join(format!(
                "{DESKTOP_ASSET_PREFIX}1.2.4{DESKTOP_ASSET_SUFFIX_ZIP}"
            ));

        fs::create_dir_all(current_archive.parent().unwrap())
            .expect("current local release directory should be created");
        fs::write(&current_archive, b"current archive")
            .expect("current local archive should be created");
        fs::create_dir_all(newer_archive.parent().unwrap())
            .expect("newer local release directory should be created");
        fs::write(&newer_archive, b"newer archive").expect("newer local archive should be created");
        fs::create_dir_all(release_root.join("v1.2.5").join(LOCAL_PUBLISHING_ZIP_DIR))
            .expect("incomplete release directory should be created");

        let update = find_newer_local_update_in_roots(std::slice::from_ref(&release_root), "1.2.3")
            .expect("local releases should be scanned")
            .expect("newer complete release should be selected");

        assert_eq!(update.version, "1.2.4");
        assert_eq!(update.archive, newer_archive);
        assert!(
            find_newer_local_update_in_roots(std::slice::from_ref(&release_root), "1.2.4")
                .expect("local releases should be scanned")
                .is_none(),
            "an incomplete release newer than the installed version should be skipped"
        );

        fs::remove_dir_all(temporary_root).expect("temporary releases should be removed");
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
        assert!(update.primary.is_zip);
    }

    #[test]
    fn accepts_exe_asset_for_transition_compatibility() {
        let releases = [release_exe("v1.2.1", "1.2.1", false)];

        let update = select_latest_desktop_release(&releases).unwrap();

        assert_eq!(update.version, "1.2.1");
        assert!(!update.primary.is_zip);
        assert!(update.primary.url.ends_with(".exe"));
    }

    #[test]
    fn prefers_zip_when_both_assets_are_present() {
        let mut release = release_zip("v1.2.1", "1.2.1", false);
        release
            .assets
            .push(release_exe("v1.2.1", "1.2.1", false).assets.remove(0));

        let update = select_latest_desktop_release(&[release]).unwrap();

        assert!(update.primary.is_zip);
        assert!(update.primary.url.ends_with(".zip"));
        assert!(
            update
                .fallback_exe
                .as_ref()
                .is_some_and(|asset| asset.url.ends_with(".exe") && !asset.is_zip)
        );
    }

    #[test]
    fn ignores_exe_fallback_without_a_verified_digest() {
        let mut release = release_zip("v1.2.1", "1.2.1", false);
        let mut exe_asset = release_exe("v1.2.1", "1.2.1", false).assets.remove(0);
        exe_asset.digest = None;
        release.assets.push(exe_asset);

        let update = select_latest_desktop_release(&[release]).unwrap();

        assert!(update.primary.is_zip);
        assert!(update.fallback_exe.is_none());
    }

    #[test]
    fn uses_same_release_exe_when_zip_cannot_be_verified() {
        let mut release = release_zip("v1.3.0", "1.3.0", false);
        release.assets[0].digest = None;
        release
            .assets
            .push(release_exe("v1.3.0", "1.3.0", false).assets.remove(0));

        let update = select_latest_desktop_release(&[release]).unwrap();

        assert_eq!(update.version, "1.3.0");
        assert!(!update.primary.is_zip);
        assert!(update.primary.url.ends_with(".exe"));
        assert!(update.fallback_exe.is_none());
    }

    #[test]
    fn skips_release_with_unusable_primary_asset_and_uses_next_valid_release() {
        let mut broken = release("v1.3.0", "1.3.0", false);
        broken.assets[0].digest = None;
        let valid = release("v1.2.1", "1.2.1", false);

        let update = select_latest_desktop_release(&[broken, valid]).unwrap();

        assert_eq!(update.version, "1.2.1");
        assert_eq!(update.release_tag, "v1.2.1");
        assert!(update.primary.is_zip);
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
    fn ignores_misnamed_asset_when_same_version_has_a_valid_asset() {
        let valid = release("v1.2.1", "1.2.1", false);
        let mut malformed = release("v1.2.1", "1.2.1", false);
        malformed.assets[0].name = "CypherSR-Desktop-GUI-v1.2.1-win-x6.zip".to_string();

        let update = select_latest_desktop_release(&[valid, malformed]).unwrap();

        assert_eq!(update.version, "1.2.1");
    }

    #[test]
    fn valid_new_release_supersedes_older_misnamed_asset() {
        let mut malformed = release("v1.2.1", "1.2.1", false);
        malformed.assets[0].name = "CypherSR-Desktop-GUI-v1.2.1-win-x6.zip".to_string();
        let current = release("v1.3.0", "1.3.0", false);

        let update = select_latest_desktop_release(&[malformed, current]).unwrap();

        assert_eq!(update.version, "1.3.0");
        assert!(update.primary.is_zip);
    }

    #[cfg(windows)]
    #[test]
    fn stages_verified_exe_when_zip_downloaded_successfully_but_is_invalid() {
        use std::net::TcpListener;
        use std::thread;
        use std::time::{SystemTime, UNIX_EPOCH};

        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be after UNIX epoch")
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "CypherSR updater fallback test {}-{unique}",
            std::process::id()
        ));
        let update_dir = root.join("update files");
        let invalid_zip = b"not a zip archive".to_vec();
        let exe_contents = b"verified executable fallback".to_vec();
        let response_bodies = [invalid_zip, exe_contents.clone()];
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind test server");
        let address = listener.local_addr().expect("get test server address");

        let server = thread::spawn(move || {
            for body in response_bodies {
                let (mut stream, _) = listener.accept().expect("accept asset request");
                let mut request = [0_u8; 2048];
                stream.read(&mut request).expect("read asset request");

                write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                )
                .expect("write response headers");
                stream.write_all(&body).expect("write asset body");
            }
        });

        let asset = |path: &str, body: &[u8], is_zip| DesktopUpdateAsset {
            url: format!("http://{address}{path}"),
            sha256: format!("{:x}", Sha256::digest(body)),
            size: body.len() as u64,
            is_zip,
        };
        let manifest = DesktopUpdateManifest {
            version: "1.2.4".to_string(),
            release_tag: "v1.2.4".to_string(),
            primary: asset("/desktop.zip", b"not a zip archive", true),
            fallback_exe: Some(asset("/desktop.exe", &exe_contents, false)),
        };
        let client = Client::builder().build().expect("build HTTP client");

        fs::create_dir_all(&update_dir).expect("create update directory");
        stage_update_assets(&client, &manifest, &update_dir).expect("stage executable fallback");
        server.join().expect("finish test server");

        assert_eq!(
            fs::read(update_dir.join(STAGED_EXE_NAME)).expect("read staged executable"),
            exe_contents
        );
        assert!(!update_dir.join(EXTRACT_SCRIPT_NAME).exists());

        fs::remove_dir_all(root).expect("remove test directory");
    }

    #[cfg(windows)]
    #[test]
    fn stops_download_when_body_exceeds_declared_size_without_content_length() {
        use std::net::TcpListener;
        use std::thread;
        use std::time::{SystemTime, UNIX_EPOCH};

        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be after UNIX epoch")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("CypherSR updater size limit test {unique}",));
        let update_dir = root.join("update files");
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind test server");
        let address = listener.local_addr().expect("get test server address");
        let response_body = vec![b'x'; 1024];
        let expected_hash = format!("{:x}", Sha256::digest(&response_body));

        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept asset request");
            let mut request = [0_u8; 2048];
            stream.read(&mut request).expect("read asset request");
            write!(stream, "HTTP/1.1 200 OK\r\nConnection: close\r\n\r\n")
                .expect("write response headers");
            stream
                .write_all(&response_body)
                .expect("write oversized response body");
        });

        let asset = DesktopUpdateAsset {
            url: format!("http://{address}/desktop.exe"),
            sha256: expected_hash,
            size: 8,
            is_zip: false,
        };
        let manifest = DesktopUpdateManifest {
            version: "1.2.4".to_string(),
            release_tag: "v1.2.4".to_string(),
            primary: asset,
            fallback_exe: None,
        };
        let client = Client::builder().build().expect("build HTTP client");

        fs::create_dir_all(&update_dir).expect("create update directory");
        let error = stage_update_assets(&client, &manifest, &update_dir)
            .expect_err("oversized download should fail");
        server.join().expect("finish test server");

        assert!(error.to_string().contains("exceeded the expected size"));
        assert!(!update_dir.join(PARTIAL_EXE_NAME).exists());
        assert!(!update_dir.join(STAGED_EXE_NAME).exists());

        fs::remove_dir_all(root).expect("remove test directory");
    }

    #[cfg(windows)]
    #[test]
    fn extracts_desktop_executable_with_spaces_and_extra_archive_content() {
        use std::time::{SystemTime, UNIX_EPOCH};

        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be after UNIX epoch")
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "CypherSR updater's test {}-{unique}",
            std::process::id()
        ));
        let source_dir = root.join("source");
        let archive_path = root.join("update archive.zip");
        let extract_dir = root.join("extracted");
        let staged_executable = root.join(STAGED_EXE_NAME);
        let create_archive_script = root.join("create-archive.ps1");
        let extract_script = root.join(EXTRACT_SCRIPT_NAME);
        let executable_contents = b"test desktop executable";

        fs::create_dir_all(&source_dir).expect("create archive source directory");
        fs::write(source_dir.join(DESKTOP_EXE_NAME), executable_contents)
            .expect("write test executable");
        fs::write(source_dir.join("additional-content.txt"), "extra")
            .expect("write additional archive content");
        fs::write(
            &create_archive_script,
            r#"
param(
    [Parameter(Mandatory = $true)][string]$SourceDirectory,
    [Parameter(Mandatory = $true)][string]$ArchivePath
)

$ErrorActionPreference = 'Stop'
Compress-Archive `
    -Path (Join-Path $SourceDirectory '*') `
    -DestinationPath $ArchivePath
"#,
        )
        .expect("write archive creation script");

        let output = Command::new("powershell.exe")
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-ExecutionPolicy",
                "Bypass",
                "-File",
            ])
            .arg(&create_archive_script)
            .arg(&source_dir)
            .arg(&archive_path)
            .output()
            .expect("run archive creation script");
        assert!(
            output.status.success(),
            "archive creation failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );

        extract_update_executable(
            &archive_path,
            &extract_dir,
            &staged_executable,
            &extract_script,
        )
        .expect("extract desktop executable");

        assert_eq!(
            fs::read(&staged_executable).expect("read staged executable"),
            executable_contents
        );
        assert!(!extract_script.exists());

        fs::remove_dir_all(root).expect("remove test directory");
    }
}
