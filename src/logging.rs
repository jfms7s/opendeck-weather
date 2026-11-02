//! The plugin's logger: simplelog at `Info`, behind a filter that keeps
//! raw inbound websocket payloads out of the log. openaction logs any event
//! it doesn't model as `Unknown event received: <raw text>`, and OpenDeck
//! sends settings (a location's coordinates) over that socket. Plugin logs
//! get pasted into bug reports, so that line keeps only its length.

use log::{LevelFilter, Log, Metadata, Record};
use simplelog::{Config, SimpleLogger};

const UNKNOWN_EVENT: &str = "Unknown event received";

/// What to log instead of `message`, or `None` to log it unchanged.
fn redact(target: &str, message: &str) -> Option<String> {
    if !target.starts_with("openaction::inbound") {
        return None;
    }
    let payload = message.strip_prefix(UNKNOWN_EVENT)?;
    let bytes = payload.strip_prefix(": ").unwrap_or(payload).len();
    Some(format!("{UNKNOWN_EVENT} (payload omitted, {bytes} bytes)"))
}

struct Redacting(Box<SimpleLogger>);

impl Log for Redacting {
    fn enabled(&self, metadata: &Metadata) -> bool {
        self.0.enabled(metadata)
    }

    fn log(&self, record: &Record) {
        // Only openaction's inbound module is formatted twice.
        let redacted = if record.target().starts_with("openaction::inbound") {
            redact(record.target(), &record.args().to_string())
        } else {
            None
        };
        match redacted {
            Some(message) => self.0.log(
                &Record::builder()
                    .args(format_args!("{message}"))
                    .level(record.level())
                    .target(record.target())
                    .module_path(record.module_path())
                    .file(record.file())
                    .line(record.line())
                    .build(),
            ),
            None => self.0.log(record),
        }
    }

    fn flush(&self) {
        self.0.flush();
    }
}

pub fn init() {
    log::set_boxed_logger(Box::new(Redacting(SimpleLogger::new(
        LevelFilter::Info,
        Config::default(),
    ))))
    .expect("logger init");
    log::set_max_level(LevelFilter::Info);
}

#[cfg(test)]
mod tests {
    use super::redact;

    #[test]
    fn an_unknown_event_loses_its_payload() {
        let line = r#"Unknown event received: {"event":"x","payload":{"latitude":38.72509}}"#;
        let out = redact("openaction::inbound", line).unwrap();
        assert_eq!(out, "Unknown event received (payload omitted, 45 bytes)");
        assert!(!out.contains("38.72509"));
        assert!(redact("openaction::inbound::mod", line).is_some());
    }

    #[test]
    fn other_lines_pass_through() {
        assert_eq!(
            redact("openaction::inbound", "Failed to handle event: closed"),
            None,
            "openaction's error lines are kept"
        );
        assert_eq!(
            redact("opendeck_weather::cache", "Unknown event received: x"),
            None
        );
    }
}
