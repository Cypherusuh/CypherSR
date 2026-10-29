use reqwest::{Url, blocking::Client};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    env,
    error::Error,
    fs::{self, File},
    io::{self, Read, Write},
    path::{Component, Path, PathBuf},
    process::{Command, ExitStatus},
    time::Duration,
};
use zip::ZipArchive;

type AppResult<T> = Result<T, Box<dyn Error>>;

const REPOSITORY: &str = "Cypherusuh/CypherSR";

const CORE_EXE: &str = "CypherSR-core.exe";
const VERSION_FILE: &str = "version.json";
const VENDOR_DIR: &str = "vendor";
const VENDOR_VERSION_FILE: &str = "vendor-version.json";

const UPDATE_TIMEOUT_SECS: u64 = 900;
const MANIFEST_TIMEOUT_SECS: u64 = 30;
const MAX_MANIFEST_BYTES: u64 = 1024 * 1024;
const MAX_UPDATE_BYTES: u64 = 1024 * 1024 * 1024;

const LEGACY_REQUIRED_PATHS: &[&str] = &[CORE_EXE, VERSION_FILE, "public", VENDOR_DIR];

const APP_UPDATE_PATHS: &[&str] = &[CORE_EXE, VERSION_FILE, "public"];

const VENDOR_UPDATE_PATHS: &[&str] = &[VENDOR_DIR];

#[derive(Debug, Deserialize)]
struct UpdateManifest {
    version: String,
    url: String,
    sha256: String,

    #[serde(default)]
    vendor: Option<String>,

    #[serde(default, rename = "vendorUrl")]
    vendor_url: Option<String>,

    #[serde(default, rename = "vendorSha256")]
    vendor_sha256: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct InstalledVersion {
    version: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct UpdateState {
    phase: String,
    version: String,

    // Empty means this is a journal written by the previous launcher.
    // Its legacy format always backed up all LEGACY_REQUIRED_PATHS.
    #[serde(default)]
    targets: Vec<String>,

    had_original: HashMap<String, bool>,
}

#[derive(Clone, Copy)]
enum UpdateKind {
    App,
    Vendor,
}

struct UpdateSpec {
    kind: UpdateKind,
    version: String,
    url: String,
    sha256: String,
}

struct AppPaths {
    root: PathBuf,
    data: PathBuf,
    temp: PathBuf,
    staging: PathBuf,
    backup: PathBuf,
    journal: PathBuf,
}

impl AppPaths {
    fn new() -> AppResult<Self> {
        let executable = env::current_exe()?;
        let root = executable
            .parent()
            .ok_or("Cannot determine launcher directory")?
            .to_path_buf();

        let temp = root.join(".tmp");

        Ok(Self {
            data: root.join("data"),
            staging: temp.join("staging"),
            backup: temp.join("backup"),
            journal: temp.join("update-state.json"),
            temp,
            root,
        })
    }
}

fn log(message: &str) {
    println!("[CypherSR] {message}");
}

fn error_message(error: &dyn Error) -> String {
    let mut message = error.to_string();
    let mut source = error.source();

    while let Some(cause) = source {
        message.push_str(": ");
        message.push_str(&cause.to_string());
        source = cause.source();
    }

    message
}

fn compare_versions(left: &str, right: &str) -> AppResult<i32> {
    let parse = |value: &str| -> AppResult<[u64; 3]> {
        let mut result = [0_u64; 3];
        let parts: Vec<&str> = value.split('.').collect();

        if parts.len() != 3 {
            return Err(format!("Invalid version: {value}").into());
        }

        for (index, part) in parts.iter().enumerate() {
            if part.is_empty() || !part.bytes().all(|byte| byte.is_ascii_digit()) {
                return Err(format!("Invalid version: {value}").into());
            }

            result[index] = part.parse()?;
        }

        Ok(result)
    };

    let left = parse(left)?;
    let right = parse(right)?;

    for index in 0..3 {
        match left[index].cmp(&right[index]) {
            std::cmp::Ordering::Greater => return Ok(1),
            std::cmp::Ordering::Less => return Ok(-1),
            std::cmp::Ordering::Equal => {}
        }
    }

    Ok(0)
}

fn valid_version(version: &str) -> bool {
    compare_versions(version, "0.0.0").is_ok()
}

fn valid_sha256(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn read_version_file(path: &Path) -> AppResult<String> {
    let content = fs::read_to_string(path)?;
    let installed: InstalledVersion = serde_json::from_str(&content)?;

    if !valid_version(&installed.version) {
        return Err(format!("Invalid installed version in {}", path.display()).into());
    }

    Ok(installed.version)
}

fn read_installed_app_version(paths: &AppPaths) -> String {
    read_version_file(&paths.root.join(VERSION_FILE)).unwrap_or_else(|_| "0.0.0".to_string())
}

fn read_installed_vendor_version(paths: &AppPaths) -> String {
    read_version_file(&paths.root.join(VENDOR_DIR).join(VENDOR_VERSION_FILE))
        .unwrap_or_else(|_| "0.0.0".to_string())
}

fn is_test_mode() -> bool {
    env::var("CYPHERSR_TEST_UPDATE")
        .map(|value| value == "1")
        .unwrap_or(false)
}

fn manifest_url() -> String {
    if is_test_mode() {
        env::var("CYPHERSR_UPDATE_MANIFEST_URL")
            .unwrap_or_else(|_| "http://127.0.0.1:8765/latest.json".to_string())
    } else {
        format!("https://github.com/{REPOSITORY}/releases/latest/download/latest.json")
    }
}

fn validate_local_test_url(url: &str) -> AppResult<()> {
    let parsed = Url::parse(url)?;

    if parsed.scheme() != "http"
        || !matches!(parsed.host_str(), Some("127.0.0.1") | Some("localhost"))
        || parsed.port().is_none()
        || parsed.username() != ""
        || parsed.password().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
    {
        return Err("Test update URLs must use HTTP on localhost with an explicit port".into());
    }

    Ok(())
}

fn validate_release_asset_url(url: &str, expected_asset: &str) -> AppResult<()> {
    let parsed = Url::parse(url)?;

    if parsed.scheme() != "https"
        || parsed.host_str() != Some("github.com")
        || parsed.port().is_some()
        || parsed.username() != ""
        || parsed.password().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
    {
        return Err("Unexpected release asset URL".into());
    }

    let expected_prefix = format!("/{REPOSITORY}/releases/download/");

    let remainder = parsed
        .path()
        .strip_prefix(&expected_prefix)
        .ok_or("Unexpected release asset URL path")?;

    let parts: Vec<&str> = remainder.split('/').collect();

    if parts.len() != 2 {
        return Err("Unexpected release asset URL path".into());
    }

    // Releases now use Desktop-version tags, independently of the
    // backend or vendor version contained in the asset filename.
    let desktop_tag_version = parts[0]
        .strip_prefix("desktop-v")
        .ok_or("Release tag must begin with desktop-v")?;

    if !valid_version(desktop_tag_version) {
        return Err("Invalid Desktop version in release tag".into());
    }

    if parts[1] != expected_asset {
        return Err(format!("Unexpected release asset filename: expected {expected_asset}").into());
    }

    Ok(())
}

fn validate_app_url(url: &str, version: &str) -> AppResult<()> {
    if is_test_mode() {
        return validate_local_test_url(url);
    }

    let expected_asset = format!("CypherSR-Backend-Update-v{version}-win-x64.zip");

    validate_release_asset_url(url, &expected_asset)
}

fn validate_vendor_url(url: &str, vendor_version: &str) -> AppResult<()> {
    if is_test_mode() {
        return validate_local_test_url(url);
    }

    let expected_asset = format!("vendor-{vendor_version}-win-x64.zip");

    validate_release_asset_url(url, &expected_asset)
}

fn client(timeout: Duration) -> AppResult<Client> {
    Ok(Client::builder()
        .timeout(timeout)
        .user_agent(concat!("CypherSR/", env!("CARGO_PKG_VERSION")))
        .build()?)
}

fn fetch_manifest() -> AppResult<UpdateManifest> {
    let mut response = client(Duration::from_secs(MANIFEST_TIMEOUT_SECS))?
        .get(manifest_url())
        .send()?
        .error_for_status()?;

    if response
        .content_length()
        .is_some_and(|length| length > MAX_MANIFEST_BYTES)
    {
        return Err("Update manifest is too large".into());
    }

    let mut bytes = Vec::new();

    (&mut response)
        .take(MAX_MANIFEST_BYTES + 1)
        .read_to_end(&mut bytes)?;

    if bytes.len() as u64 > MAX_MANIFEST_BYTES {
        return Err("Update manifest is too large".into());
    }

    let manifest: UpdateManifest = serde_json::from_slice(&bytes)?;

    if !valid_version(&manifest.version) {
        return Err("Invalid app version in latest.json".into());
    }

    if !valid_sha256(&manifest.sha256) {
        return Err("Invalid app SHA-256 checksum in latest.json".into());
    }

    validate_app_url(&manifest.url, &manifest.version)?;

    match (
        &manifest.vendor,
        &manifest.vendor_url,
        &manifest.vendor_sha256,
    ) {
        (Some(version), Some(url), Some(sha256)) => {
            if !valid_version(version) {
                return Err("Invalid vendor version in latest.json".into());
            }

            if !valid_sha256(sha256) {
                return Err("Invalid vendor SHA-256 checksum in latest.json".into());
            }

            validate_vendor_url(url, version)?;
        }
        (None, None, None) => {
            // A manifest may intentionally publish app updates only.
        }
        _ => {
            return Err("Vendor manifest fields must include vendor, vendorUrl, \
                 and vendorSha256 together"
                .into());
        }
    }

    Ok(manifest)
}

fn download_update(paths: &AppPaths, spec: &UpdateSpec) -> AppResult<PathBuf> {
    fs::create_dir_all(&paths.temp)?;

    let filename = match spec.kind {
        UpdateKind::App => "app-update.zip",
        UpdateKind::Vendor => "vendor-update.zip",
    };

    let zip_path = paths.temp.join(filename);
    let partial_path = paths.temp.join(format!("{filename}.part"));

    let _ = fs::remove_file(&zip_path);
    let _ = fs::remove_file(&partial_path);

    let kind_label = match spec.kind {
        UpdateKind::App => "app",
        UpdateKind::Vendor => "vendor",
    };

    log(&format!(
        "Downloading {kind_label} version {}...",
        spec.version
    ));

    let mut response = client(Duration::from_secs(UPDATE_TIMEOUT_SECS))?
        .get(&spec.url)
        .send()?
        .error_for_status()?;

    if response
        .content_length()
        .is_some_and(|length| length > MAX_UPDATE_BYTES)
    {
        return Err("Update archive exceeds the 1 GiB safety limit".into());
    }

    let mut output = File::create(&partial_path)?;
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
            .ok_or("Download size overflow")?;

        if total > MAX_UPDATE_BYTES {
            return Err("Update archive exceeds the 1 GiB safety limit".into());
        }

        output.write_all(&buffer[..count])?;
        hasher.update(&buffer[..count]);
    }

    output.flush()?;
    output.sync_all()?;
    drop(output);

    let actual_hash = format!("{:x}", hasher.finalize());

    if !actual_hash.eq_ignore_ascii_case(&spec.sha256) {
        return Err(
            format!("{kind_label} update SHA-256 verification failed; update rejected").into(),
        );
    }

    fs::rename(&partial_path, &zip_path)?;

    log(&format!("{kind_label} download checksum verified."));

    Ok(zip_path)
}

fn allowed_archive_path(path: &Path, kind: UpdateKind) -> bool {
    let components: Vec<_> = path.components().collect();

    if components.is_empty()
        || components
            .iter()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return false;
    }

    let first = match components[0] {
        Component::Normal(name) => name.to_string_lossy(),
        _ => return false,
    };

    match kind {
        UpdateKind::App => match first.as_ref() {
            CORE_EXE | VERSION_FILE => components.len() == 1,
            "public" => true,
            _ => false,
        },
        UpdateKind::Vendor => first == VENDOR_DIR,
    }
}

fn extract_update(paths: &AppPaths, zip_path: &Path, kind: UpdateKind) -> AppResult<()> {
    if paths.staging.exists() {
        fs::remove_dir_all(&paths.staging)?;
    }

    fs::create_dir_all(&paths.staging)?;

    let archive_file = File::open(zip_path)?;
    let mut archive = ZipArchive::new(archive_file)?;
    let mut extracted_bytes = 0_u64;

    for index in 0..archive.len() {
        let mut entry = archive.by_index(index)?;

        let relative_path = entry
            .enclosed_name()
            .ok_or("Unsafe path found in update archive")?
            .to_path_buf();

        if !allowed_archive_path(&relative_path, kind) {
            return Err(format!(
                "Unexpected path in update archive: {}",
                relative_path.display()
            )
            .into());
        }

        let output_path = paths.staging.join(&relative_path);

        if entry.is_dir() {
            fs::create_dir_all(&output_path)?;
            continue;
        }

        extracted_bytes = extracted_bytes
            .checked_add(entry.size())
            .ok_or("Extracted size overflow")?;

        if extracted_bytes > MAX_UPDATE_BYTES {
            return Err("Extracted update exceeds the 1 GiB safety limit".into());
        }

        if let Some(parent) = output_path.parent() {
            fs::create_dir_all(parent)?;
        }

        let mut output = File::create(&output_path)?;
        io::copy(&mut entry, &mut output)?;
        output.sync_all()?;
    }

    Ok(())
}

fn validate_staging(paths: &AppPaths, spec: &UpdateSpec) -> AppResult<()> {
    match spec.kind {
        UpdateKind::App => {
            for name in APP_UPDATE_PATHS {
                if !paths.staging.join(name).exists() {
                    return Err(
                        format!("App update archive is missing required item: {name}").into(),
                    );
                }
            }

            if !paths.staging.join(CORE_EXE).is_file()
                || !paths.staging.join(VERSION_FILE).is_file()
                || !paths.staging.join("public").is_dir()
                || !paths.staging.join("public/player.html").is_file()
            {
                return Err("App update archive contains invalid required items".into());
            }

            let version = read_version_file(&paths.staging.join(VERSION_FILE))?;

            if version != spec.version {
                return Err("App update ZIP version does not match its manifest".into());
            }
        }
        UpdateKind::Vendor => {
            let vendor_dir = paths.staging.join(VENDOR_DIR);
            let version_file = vendor_dir.join(VENDOR_VERSION_FILE);

            if !vendor_dir.is_dir()
                || !vendor_dir.join("yt-dlp.exe").is_file()
                || !vendor_dir.join("ffmpeg/bin/ffmpeg.exe").is_file()
                || !version_file.is_file()
            {
                return Err("Vendor update archive is missing required files".into());
            }

            let version = read_version_file(&version_file)?;

            if version != spec.version {
                return Err("Vendor ZIP version does not match its manifest".into());
            }
        }
    }

    Ok(())
}

fn copy_path(source: &Path, destination: &Path) -> AppResult<()> {
    let metadata = fs::symlink_metadata(source)?;

    if metadata.file_type().is_symlink() {
        return Err(format!("Refusing to copy symbolic link: {}", source.display()).into());
    }

    if metadata.is_dir() {
        fs::create_dir_all(destination)?;

        for entry in fs::read_dir(source)? {
            let entry = entry?;
            copy_path(&entry.path(), &destination.join(entry.file_name()))?;
        }
    } else if metadata.is_file() {
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)?;
        }

        fs::copy(source, destination)?;
    } else {
        return Err(format!("Unsupported filesystem entry: {}", source.display()).into());
    }

    Ok(())
}

fn remove_path(path: &Path) -> AppResult<()> {
    if !path.exists() {
        // Also handle broken symlinks.
        if fs::symlink_metadata(path).is_err() {
            return Ok(());
        }
    }

    let metadata = fs::symlink_metadata(path)?;

    if metadata.file_type().is_symlink() || metadata.is_file() {
        fs::remove_file(path)?;
    } else if metadata.is_dir() {
        fs::remove_dir_all(path)?;
    }

    Ok(())
}

fn write_journal(paths: &AppPaths, state: &UpdateState) -> AppResult<()> {
    fs::create_dir_all(&paths.temp)?;

    let temporary_journal = paths.temp.join("update-state.json.new");
    let bytes = serde_json::to_vec_pretty(state)?;

    {
        let mut file = File::create(&temporary_journal)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
    }

    fs::rename(&temporary_journal, &paths.journal)?;

    Ok(())
}

fn read_journal(paths: &AppPaths) -> AppResult<Option<UpdateState>> {
    if !paths.journal.exists() {
        return Ok(None);
    }

    let mut state: UpdateState = serde_json::from_slice(&fs::read(&paths.journal)?)?;

    if state.phase != "installing" {
        return Err("Unknown update recovery state; refusing cleanup".into());
    }

    // Backward compatibility with journals created by the previous
    // launcher, which always operated on all four paths.
    if state.targets.is_empty() {
        state.targets = LEGACY_REQUIRED_PATHS
            .iter()
            .map(|name| (*name).to_string())
            .collect();
    }

    for name in &state.targets {
        if !LEGACY_REQUIRED_PATHS.contains(&name.as_str()) {
            return Err(format!("Unexpected path in recovery journal: {name}").into());
        }

        if !state.had_original.contains_key(name) {
            return Err("Update recovery journal is incomplete".into());
        }
    }

    Ok(Some(state))
}

fn restore_backup(paths: &AppPaths, state: &UpdateState) -> AppResult<()> {
    log("Recovering the previous installation...");

    for name in state.targets.iter().rev() {
        let destination = paths.root.join(name);
        let backup = paths.backup.join(name);

        let had_original = *state
            .had_original
            .get(name)
            .ok_or("Missing recovery information")?;

        if had_original {
            if !backup.exists() {
                return Err(format!("Cannot recover {name}: backup is missing").into());
            }

            remove_path(&destination)?;
            copy_path(&backup, &destination)?;
        } else {
            remove_path(&destination)?;
        }
    }

    // Retain the journal if rollback fails; remove it only after the
    // entire rollback has succeeded.
    fs::remove_file(&paths.journal)?;

    log("Previous installation restored.");

    Ok(())
}

fn recover_interrupted_update(paths: &AppPaths) -> AppResult<()> {
    if let Some(state) = read_journal(paths)? {
        restore_backup(paths, &state)?;
    }

    if paths.temp.exists() {
        fs::remove_dir_all(&paths.temp)?;
    }

    Ok(())
}

fn install_update(paths: &AppPaths, spec: &UpdateSpec, zip_path: &Path) -> AppResult<()> {
    extract_update(paths, zip_path, spec.kind)?;
    validate_staging(paths, spec)?;

    if paths.backup.exists() {
        fs::remove_dir_all(&paths.backup)?;
    }

    fs::create_dir_all(&paths.backup)?;

    let targets: &[&str] = match spec.kind {
        UpdateKind::App => APP_UPDATE_PATHS,
        UpdateKind::Vendor => VENDOR_UPDATE_PATHS,
    };

    let mut had_original = HashMap::new();

    // Back up every target before changing the installed files.
    for name in targets {
        let destination = paths.root.join(name);
        let backup = paths.backup.join(name);
        let existed = fs::symlink_metadata(&destination).is_ok();

        had_original.insert((*name).to_string(), existed);

        if existed {
            copy_path(&destination, &backup)?;
        }
    }

    let state = UpdateState {
        phase: "installing".to_string(),
        version: spec.version.clone(),
        targets: targets.iter().map(|name| (*name).to_string()).collect(),
        had_original,
    };

    write_journal(paths, &state)?;

    let installation_result = (|| -> AppResult<()> {
        for name in targets {
            let destination = paths.root.join(name);
            let staged = paths.staging.join(name);

            remove_path(&destination)?;

            if !staged.exists() {
                return Err(format!("Staged update item is missing: {name}").into());
            }

            fs::rename(&staged, &destination)?;
        }

        Ok(())
    })();

    if let Err(error) = installation_result {
        log("Installation failed; restoring the previous version...");

        if let Err(rollback_error) = restore_backup(paths, &state) {
            return Err(format!(
                "Installation failed: {}. Rollback also failed: {}. \
                 Recovery files have been retained in .tmp.",
                error_message(error.as_ref()),
                error_message(rollback_error.as_ref())
            )
            .into());
        }

        return Err(error);
    }

    // The installation is committed only after the journal is removed.
    // If this fails, the next launch can still roll back safely.
    fs::remove_file(&paths.journal)?;

    let kind_label = match spec.kind {
        UpdateKind::App => "App",
        UpdateKind::Vendor => "Vendor",
    };

    log(&format!(
        "{kind_label} updated successfully to {}.",
        spec.version
    ));

    Ok(())
}

fn make_app_spec(manifest: &UpdateManifest) -> UpdateSpec {
    UpdateSpec {
        kind: UpdateKind::App,
        version: manifest.version.clone(),
        url: manifest.url.clone(),
        sha256: manifest.sha256.clone(),
    }
}

fn make_vendor_spec(manifest: &UpdateManifest) -> Option<UpdateSpec> {
    Some(UpdateSpec {
        kind: UpdateKind::Vendor,
        version: manifest.vendor.clone()?,
        url: manifest.vendor_url.clone()?,
        sha256: manifest.vendor_sha256.clone()?,
    })
}

fn check_and_apply_updates(paths: &AppPaths) -> AppResult<()> {
    let manifest = fetch_manifest()?;

    let installed_app = read_installed_app_version(paths);
    let installed_vendor = read_installed_vendor_version(paths);

    log(&format!("Installed app version: {installed_app}"));
    log(&format!("Installed vendor version: {installed_vendor}"));
    log(&format!("Latest app version: {}", manifest.version));

    let mut changed = false;

    if compare_versions(&manifest.version, &installed_app)? > 0 {
        let spec = make_app_spec(&manifest);
        log(&format!("App update available: {}", spec.version));

        let zip_path = download_update(paths, &spec)?;
        install_update(paths, &spec, &zip_path)?;
        changed = true;
    } else {
        log("App is already up to date.");
    }

    if let Some(spec) = make_vendor_spec(&manifest) {
        log(&format!("Latest vendor version: {}", spec.version));

        if compare_versions(&spec.version, &installed_vendor)? > 0 {
            log(&format!("Vendor update available: {}", spec.version));

            let zip_path = download_update(paths, &spec)?;
            install_update(paths, &spec, &zip_path)?;
            changed = true;
        } else {
            log("Vendor files are already up to date.");
        }
    } else {
        log("No vendor update is published in the manifest.");
    }

    if changed {
        log("All available updates have been installed.");
    } else {
        log("No updates are required.");
    }

    Ok(())
}

fn start_application(paths: &AppPaths) -> AppResult<ExitStatus> {
    let core_path = paths.root.join(CORE_EXE);

    if !core_path.is_file() {
        return Err(format!(
            "Cannot find {}. Download the full portable release.",
            CORE_EXE
        )
        .into());
    }

    fs::create_dir_all(&paths.data)?;

    log("Starting song-request app...");

    let mut command = Command::new(core_path);
    command
        .current_dir(&paths.root)
        .env("SONG_REQUEST_HOME", &paths.data);

    // Inherit stdout/stderr so the console displays core output.
    let mut child = command.spawn()?;
    let status = child.wait()?;

    log(&format!("App exited with status {status}."));

    Ok(status)
}

fn run() -> AppResult<()> {
    let paths = AppPaths::new()?;

    println!();
    log("Portable Twitch Song Request");
    log(&format!("App directory: {}", paths.root.display()));
    log(&format!("Data directory: {}", paths.data.display()));
    println!();

    fs::create_dir_all(&paths.data)?;

    // Never start the app or clean temporary files before recovery.
    recover_interrupted_update(&paths)?;

    let update_result = check_and_apply_updates(&paths);

    if let Err(error) = update_result {
        eprintln!(
            "[CypherSR] Update check failed: {}",
            error_message(error.as_ref())
        );

        // If a journal remains, preserve the backup and staging files.
        // The next launch must have the opportunity to recover them.
        if !paths.journal.exists() && paths.temp.exists() {
            if let Err(cleanup_error) = fs::remove_dir_all(&paths.temp) {
                eprintln!(
                    "[CypherSR] Temporary cleanup failed: {}",
                    error_message(&cleanup_error)
                );
            }
        }

        log("Continuing with the installed version, if available.");
    } else if paths.temp.exists() {
        // All requested updates completed successfully.
        fs::remove_dir_all(&paths.temp)?;
    }

    start_application(&paths)?;

    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("[CypherSR] Fatal error: {}", error_message(error.as_ref()));

        std::process::exit(1);
    }
}
