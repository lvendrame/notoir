//! Structured logging (docs/specs/35-logging-and-errors.md): JSON lines in a
//! daily rolling file under the logs directory, pretty stderr output in debug
//! builds, `NOTOIR_LOG` (EnvFilter syntax) to override the default `info`.
//! Document contents are never logged.

use std::path::Path;

use tracing_appender::non_blocking::WorkerGuard;
use tracing_appender::rolling::{RollingFileAppender, Rotation};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, Layer, fmt};

pub const LOG_ENV: &str = "NOTOIR_LOG";
pub const FILE_PREFIX: &str = "notoir";

/// Installs the global subscriber and panic hook. Keep the returned guard
/// alive for the whole process so buffered lines are flushed on exit.
pub fn init(logs_dir: &Path) -> Option<WorkerGuard> {
    let filter = || EnvFilter::try_from_env(LOG_ENV).unwrap_or_else(|_| EnvFilter::new("info"));

    let file = RollingFileAppender::builder()
        .rotation(Rotation::DAILY)
        .filename_prefix(FILE_PREFIX)
        .filename_suffix("log")
        .max_log_files(7)
        .build(logs_dir);
    let (file_layer, guard) = match file {
        Ok(appender) => {
            let (writer, guard) = tracing_appender::non_blocking(appender);
            let layer = fmt::layer()
                .json()
                .with_writer(writer)
                .with_filter(filter());
            (Some(layer), Some(guard))
        }
        Err(e) => {
            eprintln!("notoir: file logging disabled: {e}");
            (None, None)
        }
    };
    let stderr_layer = cfg!(debug_assertions).then(|| {
        fmt::layer()
            .with_writer(std::io::stderr)
            .with_filter(filter())
    });
    // A second init (tests) keeps the first subscriber.
    let _ = tracing_subscriber::registry()
        .with(file_layer)
        .with(stderr_layer)
        .try_init();

    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let backtrace = std::backtrace::Backtrace::force_capture();
        tracing::error!(panic = %info, %backtrace, "panic");
        default_hook(info);
    }));
    guard
}
