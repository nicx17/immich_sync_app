use flexi_logger::{DeferredNow, style};
use log::Record;
use std::io::Write;

/// Helper to extract formatted filename and line number from logs.
fn format_log_location(record: &Record) -> String {
    match (record.file(), record.line()) {
        (Some(file), Some(line)) => format!(" {}:{}", file, line),
        _ => String::new(),
    }
}

/// Logger formatter that produces plain text for files.
pub fn detailed_plain_format(
    w: &mut dyn Write,
    now: &mut DeferredNow,
    record: &Record,
) -> Result<(), std::io::Error> {
    write!(
        w,
        "[{}] {:<5} [{}] {}{}",
        now.format("%Y-%m-%d %H:%M:%S%.6f %:z"),
        record.level(),
        record.target(),
        record.args(),
        format_log_location(record)
    )
}

/// Logger formatter that produces ANSI color output for terminal displays.
pub fn detailed_colored_format(
    w: &mut dyn Write,
    now: &mut DeferredNow,
    record: &Record,
) -> Result<(), std::io::Error> {
    write!(
        w,
        "[{}] {} [{}] {}{}",
        now.format("%Y-%m-%d %H:%M:%S%.6f %:z"),
        style(record.level()).paint(format!("{:<5}", record.level())),
        record.target(),
        record.args(),
        format_log_location(record)
    )
}
