//! Logging: stderr in text or JSON, plus a log file in `<data dir>/logs` for the desktop
//! app (release builds on Windows have no console, so this is the only place logs and
//! panics end up).

use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{fmt, EnvFilter, Layer};

/// The log file is rotated to `scratchpad.log.1` at startup once it grows past this size.
const MAX_LOG_BYTES: u64 = 5 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum LogFormat {
    Text,
    Json,
}

/// Installs the global subscriber. Returns the log file path when one is written.
pub fn init(filter: &str, format: LogFormat, log_dir: Option<&Path>) -> Option<PathBuf> {
    let filter = || EnvFilter::try_new(filter).unwrap_or_else(|_| EnvFilter::new("info"));
    let stderr = match format {
        LogFormat::Text => fmt::layer().with_writer(std::io::stderr).boxed(),
        LogFormat::Json => fmt::layer().json().with_writer(std::io::stderr).boxed(),
    };
    let file = log_dir.and_then(|dir| match open_log_file(dir) {
        Ok(file) => Some(file),
        Err(error) => {
            eprintln!("could not open the log file in {}: {error}", dir.display());
            None
        }
    });
    let path = file.as_ref().map(|(_, path)| path.clone());
    let file_layer = file.map(|(file, _)| {
        fmt::layer()
            .with_ansi(false)
            .with_writer(Mutex::new(file))
            .with_filter(filter())
    });
    tracing_subscriber::registry()
        .with(stderr.with_filter(filter()))
        .with(file_layer)
        .init();
    install_panic_hook();
    path
}

fn open_log_file(dir: &Path) -> std::io::Result<(File, PathBuf)> {
    std::fs::create_dir_all(dir)?;
    let path = dir.join("scratchpad.log");
    if std::fs::metadata(&path).is_ok_and(|m| m.len() > MAX_LOG_BYTES) {
        let _ = std::fs::rename(&path, dir.join("scratchpad.log.1"));
    }
    let file = OpenOptions::new().create(true).append(true).open(&path)?;
    Ok((file, path))
}

/// Panics are logged (and so reach the log file) before the default hook runs.
fn install_panic_hook() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let location = info
            .location()
            .map(|l| format!("{}:{}", l.file(), l.line()))
            .unwrap_or_default();
        let message = info
            .payload()
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "unknown panic".into());
        tracing::error!(%location, "panic: {message}");
        default(info);
    }));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotates_large_log_files() {
        let dir = std::env::temp_dir().join(format!("scratchpad-logs-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("scratchpad.log"),
            vec![b'x'; (MAX_LOG_BYTES + 1) as usize],
        )
        .unwrap();
        let (_, path) = open_log_file(&dir).unwrap();
        assert_eq!(std::fs::metadata(&path).unwrap().len(), 0);
        assert!(dir.join("scratchpad.log.1").exists());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
