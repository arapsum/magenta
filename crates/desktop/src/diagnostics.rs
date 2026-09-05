use std::{
    io,
    path::{Path, PathBuf},
};

use magenta_ui::{MagentaError, Result};
use tracing_appender::{
    non_blocking::{NonBlocking, WorkerGuard},
    rolling::{RollingFileAppender, Rotation},
};
use tracing_subscriber::{fmt, layer::SubscriberExt as _, util::SubscriberInitExt as _, EnvFilter};

const DEFAULT_FILTER: &str = "magenta=info,magenta_desktop=info,magenta_ui=info,gpui=warn,gpui_component::text::format::markdown=error";
const RETAINED_LOG_FILES: usize = 7;

pub struct DiagnosticsGuard {
    _file_guard: WorkerGuard,
}

pub fn init() -> Result<DiagnosticsGuard> {
    let log_directory = log_directory()?;
    create_log_directory(&log_directory)?;
    let file_appender = build_file_appender(&log_directory)?;
    let (file_writer, file_guard) = tracing_appender::non_blocking(file_appender);
    install_subscriber(file_writer, log_directory)?;

    Ok(DiagnosticsGuard {
        _file_guard: file_guard,
    })
}

fn create_log_directory(log_directory: &Path) -> Result<()> {
    std::fs::create_dir_all(log_directory).map_err(|source| MagentaError::Diagnostics {
        path: log_directory.to_path_buf(),
        source,
    })
}

fn build_file_appender(log_directory: &Path) -> Result<RollingFileAppender> {
    RollingFileAppender::builder()
        .rotation(Rotation::DAILY)
        .filename_prefix("magenta")
        .filename_suffix("log")
        .max_log_files(RETAINED_LOG_FILES)
        .build(log_directory)
        .map_err(|source| MagentaError::Diagnostics {
            path: log_directory.to_path_buf(),
            source: io::Error::other(source),
        })
}

fn install_subscriber(file_writer: NonBlocking, log_directory: PathBuf) -> Result<()> {
    tracing_subscriber::registry()
        .with(filter())
        .with(
            fmt::layer()
                .with_ansi(false)
                .with_target(true)
                .with_writer(file_writer),
        )
        .with(
            fmt::layer()
                .with_ansi(std::io::IsTerminal::is_terminal(&std::io::stderr()))
                .with_target(true)
                .with_writer(std::io::stderr),
        )
        .try_init()
        .map_err(|source| MagentaError::Diagnostics {
            path: log_directory,
            source: io::Error::other(source),
        })
}

pub fn init_stderr() {
    if let Err(error) = tracing_subscriber::fmt()
        .with_env_filter(filter())
        .with_target(true)
        .try_init()
    {
        eprintln!("Magenta could not initialize console diagnostics: {error}");
    }
}

pub fn install_panic_hook() {
    let previous_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic_info| {
        tracing::error!(
            target: "magenta::panic",
            panic = %panic_info,
            "unexpected panic"
        );
        previous_hook(panic_info);
    }));
}

fn filter() -> EnvFilter {
    EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(DEFAULT_FILTER))
}

fn log_directory() -> Result<PathBuf> {
    dirs::data_local_dir()
        .map(|directory| directory.join("magenta").join("logs"))
        .ok_or_else(|| MagentaError::Diagnostics {
            path: PathBuf::from("<platform local data directory>"),
            source: io::Error::new(
                io::ErrorKind::NotFound,
                "the operating system did not provide a local data directory",
            ),
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_filter_is_valid() {
        EnvFilter::try_new(DEFAULT_FILTER).expect("the built-in diagnostics filter must be valid");
    }

    #[test]
    fn log_directory_is_namespaced_for_magenta() {
        if let Ok(directory) = log_directory() {
            assert!(directory.ends_with(PathBuf::from("magenta").join("logs")));
        }
    }
}
