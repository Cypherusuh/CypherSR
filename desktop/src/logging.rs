use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::thread::{self, JoinHandle};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

static LOG_FILE: OnceLock<Mutex<Option<File>>> = OnceLock::new();

pub fn initialize() -> io::Result<PathBuf> {
    let executable = std::env::current_exe()?;
    let log_directory = log_directory_for_executable(&executable)?;
    let (log_path, file) = create_log_file(&log_directory)?;

    LOG_FILE
        .set(Mutex::new(Some(file)))
        .map_err(|_| io::Error::other("Desktop logging was already initialized"))?;

    install_panic_hook();

    Ok(log_path)
}

fn log_directory_for_executable(executable: &Path) -> io::Result<PathBuf> {
    let app_directory = executable
        .parent()
        .ok_or_else(|| io::Error::other("Could not determine the desktop executable directory"))?;
    Ok(app_directory.join("logs"))
}

fn create_log_file(directory: &Path) -> io::Result<(PathBuf, File)> {
    fs::create_dir_all(directory)?;
    rotate_logs(directory)?;

    let path = directory.join("debug.log");
    let file = OpenOptions::new().create(true).append(true).open(&path)?;

    Ok((path, file))
}

fn rotate_logs(directory: &Path) -> io::Result<()> {
    let current = directory.join("debug.log");
    let previous = directory.join("debug.1.log");
    let oldest = directory.join("debug.2.log");

    remove_if_exists(&oldest)?;

    if previous.exists() {
        fs::rename(&previous, &oldest)?;
    }

    if current.exists() {
        fs::rename(&current, &previous)?;
    }

    Ok(())
}

fn remove_if_exists(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

pub fn write_stdout(arguments: fmt::Arguments<'_>) {
    write_line("front", "INFO", arguments, false);
}

pub fn write_stderr(arguments: fmt::Arguments<'_>) {
    write_line("front", "STDERR", arguments, true);
}

pub(crate) fn write_backend_stdout(message: &str) {
    write_line("back", "INFO", format_args!("{message}"), false);
}

pub(crate) fn write_backend_stderr(message: &str) {
    write_line("back", "STDERR", format_args!("{message}"), true);
}

pub(crate) fn spawn_backend_reader<R>(reader: R, stderr: bool) -> io::Result<JoinHandle<()>>
where
    R: Read + Send + 'static,
{
    thread::Builder::new()
        .name(if stderr {
            "backend-stderr-log".to_owned()
        } else {
            "backend-stdout-log".to_owned()
        })
        .spawn(move || {
            let mut reader = BufReader::new(reader);
            let mut line = Vec::new();

            loop {
                line.clear();

                match reader.read_until(b'\n', &mut line) {
                    Ok(0) => break,
                    Ok(_) => {
                        if line.last() == Some(&b'\n') {
                            line.pop();
                        }
                        if line.last() == Some(&b'\r') {
                            line.pop();
                        }

                        let message = String::from_utf8_lossy(&line);
                        if stderr {
                            write_backend_stderr(&message);
                        } else {
                            write_backend_stdout(&message);
                        }
                    }
                    Err(error) => {
                        write_backend_stderr(&format!("Could not read backend output: {error}"));
                        break;
                    }
                }
            }
        })
}

fn write_line(source: &str, level: &str, arguments: fmt::Arguments<'_>, stderr: bool) {
    let message = format!("{arguments}");
    let entry = format_log_entry(source, level, &message, current_timestamp());

    let console_result = if stderr {
        let mut console = io::stderr().lock();
        console
            .write_all(message.as_bytes())
            .and_then(|()| console.write_all(b"\n"))
    } else {
        let mut console = io::stdout().lock();
        console
            .write_all(message.as_bytes())
            .and_then(|()| console.write_all(b"\n"))
    };

    if let Err(error) = console_result {
        std::eprintln!("[Logging] Failed to write to the console: {error}");
    }

    append_log_entry(&entry);
}

fn current_timestamp() -> Duration {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
}

fn format_log_entry(source: &str, level: &str, message: &str, timestamp: Duration) -> String {
    format!(
        "[{}.{:03}] [{source}] [{level}] {message}\n",
        timestamp.as_secs(),
        timestamp.subsec_millis()
    )
}

fn append_log_entry(entry: &str) {
    let Some(log_file) = LOG_FILE.get() else {
        return;
    };

    let Ok(mut log_file) = log_file.lock() else {
        std::eprintln!("[Logging] Log file lock is poisoned.");
        return;
    };

    let Some(file) = log_file.as_mut() else {
        return;
    };

    if let Err(error) = file.write_all(entry.as_bytes()) {
        *log_file = None;
        drop(log_file);
        std::eprintln!("[Logging] Failed to write debug.log: {error}");
    }
}

fn install_panic_hook() {
    let previous_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic_info| {
        let entry = format_log_entry(
            "front",
            "ERROR",
            &format!("panic: {panic_info}"),
            current_timestamp(),
        );
        append_log_entry(&entry);
        previous_hook(panic_info);
    }));
}

#[cfg(test)]
mod tests {
    use super::{create_log_file, format_log_entry, log_directory_for_executable};
    use std::fs;
    use std::io::Write;
    use std::path::{Path, PathBuf};
    use std::time::Duration;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn log_directory_is_next_to_the_desktop_executable() {
        assert_eq!(
            log_directory_for_executable(Path::new(r"C:\CypherSR Desktop\CypherSR-Desktop.exe"))
                .expect("executable should have a parent directory"),
            PathBuf::from(r"C:\CypherSR Desktop\logs"),
        );
    }

    #[test]
    fn log_entries_identify_their_source() {
        assert_eq!(
            format_log_entry(
                "front",
                "INFO",
                "desktop started",
                Duration::from_millis(1234)
            ),
            "[1.234] [front] [INFO] desktop started\n",
        );
        assert_eq!(
            format_log_entry(
                "back",
                "STDERR",
                "backend failed",
                Duration::from_millis(1234)
            ),
            "[1.234] [back] [STDERR] backend failed\n",
        );
    }

    #[test]
    fn log_file_rotation_keeps_only_the_three_latest_runs() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be after Unix epoch")
            .as_nanos();
        let directory =
            std::env::temp_dir().join(format!("cyphersr-log-{}-{unique}", std::process::id()));
        fs::create_dir_all(&directory).expect("temporary log directory should be created");

        for generation in 1..=4 {
            let (path, mut file) =
                create_log_file(&directory).expect("log file should be created and rotated");
            writeln!(file, "generation-{generation}").expect("test log line should be written");
            drop(file);
            assert!(path.exists());
        }

        assert_eq!(
            fs::read_to_string(directory.join("debug.log"))
                .expect("current log should exist")
                .trim(),
            "generation-4",
        );
        assert_eq!(
            fs::read_to_string(directory.join("debug.1.log"))
                .expect("previous log should exist")
                .trim(),
            "generation-3",
        );
        assert_eq!(
            fs::read_to_string(directory.join("debug.2.log"))
                .expect("oldest retained log should exist")
                .trim(),
            "generation-2",
        );

        fs::remove_dir_all(directory).expect("temporary log directory should be removed");
    }
}

macro_rules! println {
    () => {
        $crate::logging::write_stdout(format_args!(""))
    };
    ($($argument:tt)*) => {
        $crate::logging::write_stdout(format_args!($($argument)*))
    };
}

macro_rules! eprintln {
    () => {
        $crate::logging::write_stderr(format_args!(""))
    };
    ($($argument:tt)*) => {
        $crate::logging::write_stderr(format_args!($($argument)*))
    };
}
