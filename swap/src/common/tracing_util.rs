use std::io;
use std::path::Path;
use std::str::FromStr;

use anyhow::Result;
use tracing_appender::rolling::{RollingFileAppender, Rotation};
use tracing_subscriber::filter::{Directive, LevelFilter};
use tracing_subscriber::fmt::time::UtcTime;
use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{fmt, EnvFilter, Layer};

use crate::cli::api::tauri_bindings::{TauriEmitter, TauriHandle, TauriLogEvent};

/// Output formats for logging messages.
pub enum Format {
    /// Standard, human readable format.
    Raw,
    /// JSON, machine readable format.
    Json,
}

/// Initialize tracing and enable logging messages according to these options.
/// Besides printing to `stdout`, this will append to a log file.
/// Said file will contain JSON-formatted logs of all levels,
/// disregarding the arguments to this function. When `trace_stdout` is `true`,
/// all tracing logs are also emitted to stdout.
pub fn init(
    level_filter: LevelFilter,
    format: Format,
    dir: impl AsRef<Path>,
    tauri_handle: Option<TauriHandle>,
    trace_stdout: bool,
) -> Result<()> {
    let ALL_CRATES: Vec<&str> = vec![
        "swap",
        "asb",
        "libp2p_community_tor",
        "unstoppableswap-gui-rs",
        "arti",
    ];
    let OUR_CRATES: Vec<&str> = vec!["swap", "asb"];

    // General log file for non-verbose logs
    let file_appender: RollingFileAppender = tracing_appender::rolling::never(&dir, "swap-all.log");

    // Verbose log file, rotated hourly, with a maximum of 24 files
    let tracing_file_appender: RollingFileAppender = RollingFileAppender::builder()
        .rotation(Rotation::HOURLY)
        .filename_prefix("tracing")
        .filename_suffix("log")
        .max_log_files(24)
        .build(&dir)
        .expect("initializing rolling file appender failed");

    // Layer for writing to the general log file
    // Crates: swap, asb
    // Level: Passed in
    let file_layer = fmt::layer()
        .with_writer(file_appender)
        .with_ansi(false)
        .with_timer(UtcTime::rfc_3339())
        .with_target(false)
        .json()
        .with_filter(env_filter(level_filter, OUR_CRATES.clone())?);

    // Layer for writing to the verbose log file
    // Crates: swap, asb, libp2p_community_tor, unstoppableswap-gui-rs, arti (all relevant crates)
    // Level: TRACE
    let tracing_file_layer = fmt::layer()
        .with_writer(tracing_file_appender)
        .with_ansi(false)
        .with_timer(UtcTime::rfc_3339())
        .with_target(false)
        .json()
        .with_filter(env_filter(LevelFilter::TRACE, ALL_CRATES.clone())?);

    // Layer for writing to the terminal
    // Crates: swap, asb
    // Level: Passed in
    let is_terminal = atty::is(atty::Stream::Stderr);
    let terminal_layer = fmt::layer()
        .with_writer(std::io::stdout)
        .with_ansi(is_terminal)
        .with_timer(UtcTime::rfc_3339())
        .with_target(false);

    // Layer for writing to the Tauri guest. This will be displayed in the GUI.
    // Crates: swap, asb, libp2p_community_tor, unstoppableswap-gui-rs, arti
    // Level: Passed in
    // Note: Include arti logs in Tauri to show Tor progress in GUI
    let tauri_crates: Vec<&str> = vec![
        "swap",
        "asb", 
        "libp2p_community_tor",
        "unstoppableswap-gui-rs",
        "arti",
    ];
    let tauri_layer = fmt::layer()
        .with_writer(TauriWriter::new(tauri_handle))
        .with_ansi(false)
        .with_timer(UtcTime::rfc_3339())
        .with_target(true)
        .json()
        .with_filter(env_filter(level_filter, tauri_crates)?);

    // If trace_stdout is true, we log all messages to the terminal
    // Otherwise, we only log our own log messages (no arti logs)
    let terminal_layer_env_filter = match trace_stdout {
        true => env_filter(LevelFilter::TRACE, ALL_CRATES.clone())?,
        false => {
            // For normal operation, only log our own crates (exclude arti completely)
            env_filter(level_filter, OUR_CRATES.clone())?
        }
    };
    let final_terminal_layer = match format {
        Format::Json => terminal_layer
            .json()
            .with_filter(terminal_layer_env_filter)
            .boxed(),
        Format::Raw => terminal_layer
            .with_filter(terminal_layer_env_filter)
            .boxed(),
    };

    let subscriber = tracing_subscriber::registry()
        .with(file_layer)
        .with(tracing_file_layer)
        .with(final_terminal_layer)
        .with(tauri_layer);

    subscriber.try_init()?;

    // Now we can use the tracing macros to log messages
    tracing::info!(%level_filter, logs_dir=%dir.as_ref().display(), "Initialized tracing. General logs will be written to swap-all.log, and verbose logs to tracing*.log");

    Ok(())
}

/// This function controls which crate's logs actually get logged and from which level.
fn env_filter(level_filter: LevelFilter, crates: Vec<&str>) -> Result<EnvFilter> {
    let mut filter = EnvFilter::from_default_env();

    // Add directives for each crate in the provided list
    for crate_name in crates {
        filter = filter.add_directive(Directive::from_str(&format!(
            "{}={}",
            crate_name, &level_filter
        ))?);
    }

    Ok(filter)
}

/// A writer that forwards tracing log messages to the tauri guest.
#[derive(Clone)]
pub struct TauriWriter {
    tauri_handle: Option<TauriHandle>,
}

impl TauriWriter {
    /// Create a new Tauri writer that sends log messages to the tauri guest.
    pub fn new(tauri_handle: Option<TauriHandle>) -> Self {
        Self { tauri_handle }
    }
}

/// This is needed for tracing to accept this as a writer.
impl<'a> MakeWriter<'a> for TauriWriter {
    type Writer = TauriWriter;

    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

/// For every write issued by tracing we simply pass the string on as an event to the tauri guest.
impl std::io::Write for TauriWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        // Since this function accepts bytes, we need to pass to utf8 first
        let owned_buf = buf.to_owned();
        let utf8_string = String::from_utf8(owned_buf)
            .map_err(|err| io::Error::new(io::ErrorKind::InvalidInput, err))?;

        // Then send to tauri
        self.tauri_handle.emit_cli_log_event(TauriLogEvent {
            buffer: utf8_string,
        });

        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        // No-op, we don't need to flush anything
        Ok(())
    }
}
