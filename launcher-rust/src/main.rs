use reqwest::blocking::Client;
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
const UPDATE_ASSET: &str = "CypherSR-update-win-x64.zip";

const UPDATE_TIMEOUT_SECS: u64 = 900;
const MANIFEST_TIMEOUT_SECS: u64 = 30;
const MAX_MANIFEST_BYTES: u64 = 1024 * 1024;
const MAX_UPDATE_BYTES: u64 = 1024 * 1024 * 1024;

const REQUIRED_PATHS: &[&str] = &[CORE_EXE, VERSION_FILE, "public", "vendor"];

#[derive(Debug, Deserialize)]
struct UpdateManifest {
    version: String,
    url: String,
    sha256: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct InstalledVersion {
    version: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct UpdateState {
    phase: String,
    version: String,
    had_original: HashMap<String, bool>,
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

        Ok(Self {
            data: root.join("data"),
            temp: root.join(".tmp"),
            staging: root.join(".tmp").join("staging"),
            backup: root.join(".tmp").join("backup"),
            journal: root.join(".tmp").join("update-state.json"),
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
        let mut result = [0; 3];
        let parts: Vec<&str> = value.split('.').collect();

        if parts.len() != 3 {
            return Err(format!("Invalid version: {value}").into());
        }

        for (index, part) in parts.iter().enumerate() {
            if part.is_empty() || !part.bytes().all(|b| b.is_ascii_digit()) {
                return Err(format!("Invalid version: {value}").into());
            }

            result[index] = part.parse()?;
        }

        Ok(result)
    };

    let a = parse(left)?;
    let b = parse(right)?;

    for index in 0..3 {
        if a[index] > b[index] {
            return Ok(1);
        }

        if a[index] < b[index] {
            return Ok(-1);
        }
    }

    Ok(0)
}

fn valid_version(version: &str) -> bool {
    compare_versions(version, "0.0.0").is_ok()
}

fn read_installed_version(paths: &AppPaths) -> String {
    let result = (|| -> AppResult<String> {
        let content = fs::read_to_string(paths.root.join(VERSION_FILE))?;
        let version: InstalledVersion = serde_json::from_str(&content)?;

        if !valid_version(&version.version) {
            return Err("Installed version is invalid".into());
        }

        Ok(version.version)
    })();

    result.unwrap_or_else(|_| "0.0.0".to_string())
}

fn is_test_mode() -> bool {
    env::var("CYPHERSR_TEST_UPDATE")
        .map(|value| value == "1")
        .unwrap_or(false)
}

fn manifest_url() -> String {
    if is_test_mode() {
        env::var("CYPHERSR_UPDATE_MANIFEST_URL")
            .unwrap_or_else(|_| {
                "http://127.0.0.1:8765/latest.json".to_string()
            })
    } else {
        format!(
            "https://github.com/{REPOSITORY}/releases/latest/download/latest.json"
        )
    }
}

fn validate_update_url(url: &str) -> AppResult<()> {
    if is_test_mode() {
        let parsed = reqwest::Url::parse(url)?;

        if parsed.scheme() != "http"
            || !matches!(
                parsed.host_str(),
                Some("127.0.0.1") | Some("localhost")
            )
        {
            return Err(
                "Test updates must use an HTTP localhost URL".into(),
            );
        }
    } else {
        let expected = format!(
            "https://github.com/{REPOSITORY}/releases/latest/download/{UPDATE_ASSET}"
        );

        if url != expected {
            return Err("Unexpected update download URL".into());
        }
    }

    Ok(())
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
    let mut bytes = Vec::new();

    (&mut response)
        .take(MAX_MANIFEST_BYTES + 1)
        .read_to_end(&mut bytes)?;

    if bytes.len() as u64 > MAX_MANIFEST_BYTES {
        return Err("Update manifest is too large".into());
    }

    let manifest: UpdateManifest = serde_json::from_slice(&bytes)?;

    if !valid_version(&manifest.version) {
        return Err("Invalid version in latest.json".into());
    }

    if !manifest.sha256.bytes().all(|byte| byte.is_ascii_hexdigit()) || manifest.sha256.len() != 64
    {
        return Err("Invalid SHA-256 checksum in latest.json".into());
    }

    validate_update_url(&manifest.url)?;

    Ok(manifest)
}

fn download_update(paths: &AppPaths, manifest: &UpdateManifest) -> AppResult<PathBuf> {
    fs::create_dir_all(&paths.temp)?;

    let zip_path = paths.temp.join("update.zip");
    let partial_path = paths.temp.join("update.zip.part");

    let _ = fs::remove_file(&zip_path);
    let _ = fs::remove_file(&partial_path);

    log(&format!("Downloading version {}...", manifest.version));

    let mut response = client(Duration::from_secs(UPDATE_TIMEOUT_SECS))?
        .get(&manifest.url)
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

    if !actual_hash.eq_ignore_ascii_case(&manifest.sha256) {
        return Err("SHA-256 verification failed; update rejected".into());
    }

    fs::rename(&partial_path, &zip_path)?;

    log("Download checksum verified.");

    Ok(zip_path)
}

fn is_allowed_archive_path(path: &Path) -> bool {
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

    match first.as_ref() {
        CORE_EXE | VERSION_FILE => components.len() == 1,
        "public" | "vendor" => true,
        _ => false,
    }
}

fn extract_update(paths: &AppPaths, zip_path: &Path) -> AppResult<()> {
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

        if !is_allowed_archive_path(&relative_path) {
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

fn validate_staging(paths: &AppPaths, manifest: &UpdateManifest) -> AppResult<()> {
    for name in REQUIRED_PATHS {
        if !paths.staging.join(name).exists() {
            return Err(format!("Update archive is missing required item: {name}").into());
        }
    }

    if !paths.staging.join(CORE_EXE).is_file()
        || !paths.staging.join(VERSION_FILE).is_file()
        || !paths.staging.join("public").is_dir()
        || !paths.staging.join("vendor").is_dir()
    {
        return Err("Update archive contains invalid required items".into());
    }

    let version: InstalledVersion =
        serde_json::from_str(&fs::read_to_string(paths.staging.join(VERSION_FILE))?)?;

    if version.version != manifest.version {
        return Err("Update ZIP version does not match its manifest".into());
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
        return Ok(());
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

    // The journal is created before installation starts. It is not
    // replaced while installation is in progress.
    fs::rename(&temporary_journal, &paths.journal)?;

    Ok(())
}

fn read_journal(paths: &AppPaths) -> AppResult<Option<UpdateState>> {
    if !paths.journal.exists() {
        return Ok(None);
    }

    let state: UpdateState = serde_json::from_slice(&fs::read(&paths.journal)?)?;

    if state.phase != "installing" {
        return Err("Unknown update recovery state; refusing cleanup".into());
    }

    for name in REQUIRED_PATHS {
        if !state.had_original.contains_key(*name) {
            return Err("Update recovery journal is incomplete".into());
        }
    }

    Ok(Some(state))
}

fn restore_backup(paths: &AppPaths, state: &UpdateState) -> AppResult<()> {
    log("Recovering the previous installation...");

    for name in REQUIRED_PATHS.iter().rev() {
        let destination = paths.root.join(name);
        let backup = paths.backup.join(name);
        let had_original = *state
            .had_original
            .get(*name)
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

    // Remove the journal only after rollback is complete. If rollback
    // is interrupted, the next launch can retry using the same backups.
    fs::remove_file(&paths.journal)?;

    log("Previous installation restored.");

    Ok(())
}

fn recover_interrupted_update(paths: &AppPaths) -> AppResult<()> {
    match read_journal(paths)? {
        Some(state) => restore_backup(paths, &state)?,
        None => {}
    }

    // The journal is removed only after successful recovery.
    // At this point, no temporary files are needed.
    if paths.temp.exists() {
        fs::remove_dir_all(&paths.temp)?;
    }

    Ok(())
}

fn install_update(paths: &AppPaths, manifest: &UpdateManifest, zip_path: &Path) -> AppResult<()> {
    extract_update(paths, zip_path)?;
    validate_staging(paths, manifest)?;

    if paths.backup.exists() {
        fs::remove_dir_all(&paths.backup)?;
    }

    fs::create_dir_all(&paths.backup)?;

    let mut had_original = HashMap::new();

    // Copy all originals first. The installed application remains
    // untouched until every backup has been made successfully.
    for name in REQUIRED_PATHS {
        let destination = paths.root.join(name);
        let backup = paths.backup.join(name);
        let existed = destination.exists();

        had_original.insert((*name).to_string(), existed);

        if existed {
            copy_path(&destination, &backup)?;
        }
    }

    let state = UpdateState {
        phase: "installing".to_string(),
        version: manifest.version.clone(),
        had_original,
    };

    // From this point forward, the next launch must recover if this
    // process is interrupted.
    write_journal(paths, &state)?;

    let installation_result = (|| -> AppResult<()> {
        for name in REQUIRED_PATHS {
            let destination = paths.root.join(name);
            let staged = paths.staging.join(name);

            remove_path(&destination)?;
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

    // The complete new installation is in place. Removing the journal
    // commits the update; leftover .tmp files can now be discarded.
    fs::remove_file(&paths.journal)?;

    log(&format!("Updated successfully to {}.", manifest.version));

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

    // Inherit stdout/stderr so the launcher console displays the
    // core application's output.
    #[cfg(windows)]
    {
        const CREATE_NO_WINDOW: u32 = 0x08000000;

        // Do not use CREATE_NO_WINDOW: the core app should keep its
        // normal console behavior.
        let _ = CREATE_NO_WINDOW;
    }

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

    // Recover before removing temporary files or starting the core.
    recover_interrupted_update(&paths)?;

    let installed_version = read_installed_version(&paths);
    log(&format!("Installed version: {installed_version}"));

    let update_result = (|| -> AppResult<()> {
        let manifest = fetch_manifest()?;

        if compare_versions(&manifest.version, &installed_version)? <= 0 {
            log("Already up to date.");
            return Ok(());
        }

        log(&format!("New version available: {}", manifest.version));

        let zip_path = download_update(&paths, &manifest)?;
        install_update(&paths, &manifest, &zip_path)?;

        // Safe only after a successful commit.
        if paths.temp.exists() {
            fs::remove_dir_all(&paths.temp)?;
        }

        Ok(())
    })();

    if let Err(error) = update_result {
        eprintln!(
            "[CypherSR] Update check failed: {}",
            error_message(error.as_ref())
        );

        // If a journal remains, preserve backups for the next launch.
        // Never erase files needed for recovery.
        if !paths.journal.exists() && paths.temp.exists() {
            if let Err(cleanup_error) = fs::remove_dir_all(&paths.temp) {
                eprintln!(
                    "[CypherSR] Temporary cleanup failed: {}",
                    error_message(&cleanup_error)
                );
            }
        }

        log("Continuing with the installed version, if available.");
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
