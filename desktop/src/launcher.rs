use std::io;
use std::mem::{size_of, zeroed};
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle, RawHandle};
use std::os::windows::process::CommandExt;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::thread::JoinHandle;

use windows_sys::Win32::Foundation::{
    CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE, INVALID_HANDLE_VALUE,
};
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, TH32CS_SNAPTHREAD, THREADENTRY32, Thread32First, Thread32Next,
};
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
    SetInformationJobObject,
};
use windows_sys::Win32::System::Threading::{
    CREATE_SUSPENDED, CreateMutexW, OpenThread, ResumeThread, THREAD_SUSPEND_RESUME,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{MB_ICONERROR, MB_OK, MessageBoxW};

const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const PATCHER_EXE: &str = "CypherSR-Updater.exe";
const BACKEND_DIRECTORY: &str = "backend";
const DATA_DIRECTORY: &str = "data";
const SONG_REQUEST_HOME_ENV: &str = "SONG_REQUEST_HOME";
const SONG_REQUEST_RUNTIME_HOME_ENV: &str = "SONG_REQUEST_RUNTIME_HOME";
#[cfg(not(feature = "local-updater-test"))]
const SINGLE_INSTANCE_MUTEX: &str = "Local\\CypherSR.Desktop.SingleInstance";
#[cfg(feature = "local-updater-test")]
const SINGLE_INSTANCE_MUTEX: &str = "Local\\CypherSR.Desktop.LocalUpdaterTest.SingleInstance";

/// Keeps the updater and its backend process alive for the desktop lifetime.
pub struct BackendPatcher {
    _child: Child,
    job: Option<OwnedHandle>,
    output_threads: Vec<JoinHandle<()>>,
}

impl Drop for BackendPatcher {
    fn drop(&mut self) {
        // Closing the job stops backend processes before their output readers are joined.
        self.job.take();
        join_backend_output_threads(std::mem::take(&mut self.output_threads));
    }
}

pub fn acquire_single_instance() -> io::Result<OwnedHandle> {
    let mutex_name: Vec<u16> = SINGLE_INSTANCE_MUTEX
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();

    let handle = unsafe { CreateMutexW(std::ptr::null(), 0, mutex_name.as_ptr()) };

    if handle.is_null() {
        return Err(io::Error::last_os_error());
    }

    let last_error = unsafe { GetLastError() };
    if last_error == ERROR_ALREADY_EXISTS {
        unsafe {
            CloseHandle(handle);
        }

        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "Another CypherSR Desktop instance is already running.",
        ));
    }

    Ok(unsafe { OwnedHandle::from_raw_handle(handle as RawHandle) })
}

pub fn register_bundled_mpv_if_needed() -> Result<(), Box<dyn std::error::Error>> {
    let executable = std::env::current_exe()?;
    let app_dir = executable
        .parent()
        .ok_or_else(|| io::Error::other("Could not determine the desktop executable directory"))?;
    let backend_mpv_dir = app_dir.join(BACKEND_DIRECTORY).join("vendor").join("mpv");
    let legacy_mpv_dir = app_dir.join("vendor").join("mpv");
    let mpv_dir = if backend_mpv_dir.join("mpv.exe").is_file() {
        backend_mpv_dir
    } else {
        legacy_mpv_dir
    };
    let mpv_executable = mpv_dir.join("mpv.exe");
    let register_script = mpv_dir.join("mpv-register.bat");

    if !mpv_executable.exists() && !register_script.exists() {
        println!("[MPV] No bundled MPV found; skipping registration.");
        return Ok(());
    }

    for path in [&mpv_executable, &register_script] {
        if !path.is_file() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("Bundled MPV file was not found: {}", path.display()),
            )
            .into());
        }
    }

    let data_dir = app_dir.join(BACKEND_DIRECTORY).join(DATA_DIRECTORY);
    std::fs::create_dir_all(&data_dir)?;

    let registration_marker = data_dir.join("mpv-registration-path.txt");
    let mpv_path = mpv_executable.to_string_lossy();
    let previous_registration = match std::fs::read_to_string(&registration_marker) {
        Ok(path) => Some(path),
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => return Err(error.into()),
    };

    if previous_registration
        .as_deref()
        .map(str::trim)
        .is_some_and(|path| path.eq_ignore_ascii_case(&mpv_path))
    {
        return Ok(());
    }

    let output = Command::new("cmd.exe")
        .args(["/D", "/C"])
        .arg(&register_script)
        .current_dir(&mpv_dir)
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|error| {
            io::Error::new(
                error.kind(),
                format!(
                    "Could not run MPV registration script {}: {error}",
                    register_script.display()
                ),
            )
        })?;

    if !output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        let details = [stdout.trim(), stderr.trim()]
            .into_iter()
            .filter(|text| !text.is_empty())
            .collect::<Vec<_>>()
            .join("\n");

        return Err(io::Error::other(format!(
            "MPV registration script exited with {}.\n{}",
            output.status, details
        ))
        .into());
    }

    std::fs::write(&registration_marker, mpv_path.as_bytes())?;
    println!("[MPV] Registered {}", mpv_executable.display());

    Ok(())
}

pub fn launch_backend_patcher() -> Result<BackendPatcher, Box<dyn std::error::Error>> {
    let executable = std::env::current_exe()?;
    let app_dir = executable
        .parent()
        .ok_or_else(|| io::Error::other("Could not determine the desktop executable directory"))?;

    let backend_dir = app_dir.join(BACKEND_DIRECTORY);
    let bundled_patcher = backend_dir.join(PATCHER_EXE);
    let (patcher_path, runtime_dir) = if bundled_patcher.is_file() {
        (bundled_patcher, backend_dir.clone())
    } else {
        (app_dir.join(PATCHER_EXE), app_dir.to_path_buf())
    };

    if !patcher_path.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!(
                "Patcher executable was not found: {}",
                patcher_path.display()
            ),
        )
        .into());
    }

    let data_dir = backend_dir.join(DATA_DIRECTORY);
    std::fs::create_dir_all(&data_dir).map_err(|error| {
        io::Error::new(
            error.kind(),
            format!(
                "Could not create portable data directory {}: {error}",
                data_dir.display()
            ),
        )
    })?;

    let legacy_data_dir = app_dir.join(DATA_DIRECTORY);
    if legacy_data_dir != data_dir && legacy_data_dir.is_dir() {
        println!(
            "[Launcher] Merging legacy data from {} into {}",
            legacy_data_dir.display(),
            data_dir.display()
        );
        merge_legacy_data(&legacy_data_dir, &data_dir)?;
    }

    println!("[Launcher] Updater: {}", patcher_path.display());
    println!("[Launcher] Data directory: {}", data_dir.display());

    let job_handle = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
    if job_handle.is_null() {
        return Err(io::Error::other(format!(
            "CreateJobObjectW failed: {}",
            io::Error::last_os_error()
        ))
        .into());
    }

    let job = unsafe { OwnedHandle::from_raw_handle(job_handle as RawHandle) };
    let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { zeroed() };
    limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;

    let configured = unsafe {
        SetInformationJobObject(
            job_handle,
            JobObjectExtendedLimitInformation,
            &limits as *const _ as *const _,
            size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        )
    };

    if configured == 0 {
        return Err(io::Error::other(format!(
            "SetInformationJobObject failed: {}",
            io::Error::last_os_error()
        ))
        .into());
    }

    let mut child = Command::new(&patcher_path)
        .current_dir(&runtime_dir)
        .env(SONG_REQUEST_HOME_ENV, &data_dir)
        .env(SONG_REQUEST_RUNTIME_HOME_ENV, &runtime_dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .creation_flags(CREATE_SUSPENDED | CREATE_NO_WINDOW)
        .spawn()
        .map_err(|error| {
            io::Error::new(
                error.kind(),
                format!(
                    "Could not spawn updater {}: {error}",
                    patcher_path.display()
                ),
            )
        })?;

    println!("[Launcher] Updater created suspended (PID {}).", child.id());

    let stdout = match child.stdout.take() {
        Some(stdout) => stdout,
        None => {
            terminate_child(&mut child);
            return Err(io::Error::other("Updater stdout pipe was not created").into());
        }
    };
    let stderr = match child.stderr.take() {
        Some(stderr) => stderr,
        None => {
            terminate_child(&mut child);
            return Err(io::Error::other("Updater stderr pipe was not created").into());
        }
    };
    let stdout_thread = match crate::logging::spawn_backend_reader(stdout, false) {
        Ok(thread) => thread,
        Err(error) => {
            terminate_child(&mut child);
            return Err(error.into());
        }
    };
    let stderr_thread = match crate::logging::spawn_backend_reader(stderr, true) {
        Ok(thread) => thread,
        Err(error) => {
            terminate_child(&mut child);
            join_backend_output_threads(vec![stdout_thread]);
            return Err(error.into());
        }
    };
    let output_threads = vec![stdout_thread, stderr_thread];

    let assigned = unsafe { AssignProcessToJobObject(job_handle, child.as_raw_handle() as HANDLE) };
    if assigned == 0 {
        let error = io::Error::other(format!(
            "AssignProcessToJobObject failed: {}",
            io::Error::last_os_error()
        ));
        terminate_child(&mut child);
        join_backend_output_threads(output_threads);
        return Err(error.into());
    }

    println!("[Launcher] Updater assigned to Job Object.");

    if let Err(error) = resume_suspended_process(child.id()) {
        terminate_child(&mut child);
        join_backend_output_threads(output_threads);
        return Err(error.into());
    }
    println!("[Launcher] Updater resumed successfully.");

    Ok(BackendPatcher {
        _child: child,
        job: Some(job),
        output_threads,
    })
}

fn join_backend_output_threads(threads: Vec<JoinHandle<()>>) {
    for thread in threads {
        if thread.join().is_err() {
            crate::logging::write_backend_stderr("Backend output reader thread panicked.");
        }
    }
}

fn merge_legacy_data(source: &Path, destination: &Path) -> io::Result<()> {
    std::fs::create_dir_all(destination)?;

    for entry in std::fs::read_dir(source)? {
        let entry = entry?;
        let source_path = entry.path();
        let destination_path = destination.join(entry.file_name());
        let source_type = entry.file_type()?;
        let destination_type = std::fs::symlink_metadata(&destination_path);

        if source_type.is_dir() {
            match destination_type {
                Ok(metadata) if metadata.is_dir() => {
                    merge_legacy_data(&source_path, &destination_path)?;
                }
                Ok(_) => continue,
                Err(error) if error.kind() == io::ErrorKind::NotFound => {
                    std::fs::rename(&source_path, &destination_path)?;
                }
                Err(error) => return Err(error),
            }
        } else if source_type.is_file() {
            match destination_type {
                Ok(_) => continue,
                Err(error) if error.kind() == io::ErrorKind::NotFound => {
                    std::fs::rename(&source_path, &destination_path)?;
                }
                Err(error) => return Err(error),
            }
        }
    }

    match std::fs::remove_dir(source) {
        Ok(()) => Ok(()),
        Err(error)
            if matches!(
                error.kind(),
                io::ErrorKind::DirectoryNotEmpty | io::ErrorKind::NotFound
            ) =>
        {
            Ok(())
        }
        Err(error) => Err(error),
    }
}

fn resume_suspended_process(process_id: u32) -> io::Result<()> {
    unsafe {
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0);
        if snapshot == INVALID_HANDLE_VALUE {
            return Err(io::Error::other(format!(
                "CreateToolhelp32Snapshot failed: {}",
                io::Error::last_os_error()
            )));
        }

        let mut entry: THREADENTRY32 = zeroed();
        entry.dwSize = size_of::<THREADENTRY32>() as u32;
        if Thread32First(snapshot, &mut entry) == 0 {
            let error = io::Error::last_os_error();
            CloseHandle(snapshot);
            return Err(io::Error::other(format!("Thread32First failed: {error}")));
        }

        loop {
            if entry.th32OwnerProcessID == process_id {
                let thread_id = entry.th32ThreadID;
                let thread_handle = OpenThread(THREAD_SUSPEND_RESUME, 0, thread_id);
                if thread_handle.is_null() {
                    let error = io::Error::last_os_error();
                    CloseHandle(snapshot);
                    return Err(io::Error::other(format!(
                        "OpenThread failed for updater PID {process_id}, thread ID {thread_id}: {error}"
                    )));
                }

                let resume_result = ResumeThread(thread_handle);
                let resume_error = if resume_result == u32::MAX {
                    Some(io::Error::last_os_error())
                } else {
                    None
                };
                CloseHandle(thread_handle);
                CloseHandle(snapshot);

                return match resume_error {
                    Some(error) => Err(io::Error::other(format!(
                        "ResumeThread failed for updater PID {process_id}, thread ID {thread_id}: {error}"
                    ))),
                    None => Ok(()),
                };
            }

            entry.dwSize = size_of::<THREADENTRY32>() as u32;
            if Thread32Next(snapshot, &mut entry) == 0 {
                let error = io::Error::last_os_error();
                CloseHandle(snapshot);
                return Err(io::Error::other(format!(
                    "Could not find a thread for updater PID {process_id}: {error}"
                )));
            }
        }
    }
}

fn terminate_child(child: &mut Child) {
    if let Err(error) = child.kill() {
        eprintln!("[Launcher] Failed to terminate updater: {error}");
    }
    if let Err(error) = child.wait() {
        eprintln!("[Launcher] Failed to wait for updater: {error}");
    }
}

pub fn show_error_dialog(message: &str) {
    let title: Vec<u16> = "CypherSR Desktop".encode_utf16().chain(Some(0)).collect();
    let message: Vec<u16> = message.encode_utf16().chain(Some(0)).collect();

    unsafe {
        MessageBoxW(
            std::ptr::null_mut(),
            message.as_ptr(),
            title.as_ptr(),
            MB_OK | MB_ICONERROR,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::merge_legacy_data;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn legacy_data_is_merged_without_overwriting_existing_backend_data() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be after UNIX epoch")
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "cypersr-data-migration-{}-{unique}",
            std::process::id()
        ));
        let source = root.join("data");
        let destination = root.join("backend").join("data");

        fs::create_dir_all(&source).expect("create legacy data directory");
        fs::create_dir_all(&destination).expect("create backend data directory");
        fs::write(source.join("config.json"), "legacy").expect("write legacy config");
        fs::write(destination.join("config.json"), "backend").expect("write backend config");
        fs::write(source.join("twitch-tokens.json"), "legacy token").expect("write legacy token");

        merge_legacy_data(&source, &destination).expect("merge legacy data");

        assert_eq!(
            fs::read_to_string(destination.join("config.json")).expect("read backend config"),
            "backend"
        );
        assert_eq!(
            fs::read_to_string(destination.join("twitch-tokens.json"))
                .expect("read migrated token"),
            "legacy token"
        );
        assert_eq!(
            fs::read_to_string(source.join("config.json"))
                .expect("legacy conflict should be preserved"),
            "legacy"
        );

        fs::remove_dir_all(root).expect("remove test directories");
    }
}
