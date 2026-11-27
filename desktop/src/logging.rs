use directories::BaseDirs;
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

static LOG_FILE: OnceLock<Mutex<Option<File>>> = OnceLock::new();

pub fn initialize() -> io::Result<PathBuf> {
    let base_dirs = BaseDirs::new()
        .ok_or_else(|| io::Error::other("Could not determine the local app data directory"))?;
    let log_directory = base_dirs.data_local_dir().join("CypherSR").join("Desktop");
    let (log_path, file) = create_log_file(&log_directory)?;

    LOG_FILE
        .set(Mutex::new(Some(file)))
        .map_err(|_| io::Error::other("Desktop logging was already initialized"))?;

    install_panic_hook();

    Ok(log_path)
}

fn create_log_file(directory: &Path) -> io::Result<(PathBuf, File)> {
    fs::create_dir_all(directory)?;
    rotate_logs(directory)?;

    let path = directory.join("debug.log");
    let file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(&path)?;

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
    write_line("INFO", arguments, false);
}

pub fn write_stderr(arguments: fmt::Arguments<'_>) {
    write_line("STDERR", arguments, true);
}

fn write_line(level: &str, arguments: fmt::Arguments<'_>, stderr: bool) {
    let message = format!("{arguments}");
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let entry = format!(
        "[{}.{:03}] [{level}] {message}\n",
        timestamp.as_secs(),
        timestamp.subsec_millis()
    );

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
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();
        let entry = format!(
            "[{}.{:03}] [ERROR] panic: {panic_info}\n",
            timestamp.as_secs(),
            timestamp.subsec_millis()
        );
        append_log_entry(&entry);
        previous_hook(panic_info);
    }));
}

#[cfg(test)]
mod tests {
    use super::create_log_file;
    use std::fs;
    use std::io::Write;
    use std::time::{SystemTime, UNIX_EPOCH};

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
