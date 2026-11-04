//! Logging to stderr. Under systemd (`JOURNAL_STREAM` set) each line gets a
//! syslog priority prefix that journald understands; otherwise a plain level.
//! Never pass the token or report contents here.

use std::sync::OnceLock;

fn journald() -> bool {
    static J: OnceLock<bool> = OnceLock::new();
    *J.get_or_init(|| std::env::var_os("JOURNAL_STREAM").is_some())
}

fn line(priority: u8, level: &str, msg: &str) {
    if journald() {
        eprintln!("<{priority}>{msg}");
    } else {
        eprintln!("{level} {msg}");
    }
}

pub fn info(msg: &str) {
    line(6, "INFO", msg);
}

pub fn warn(msg: &str) {
    line(4, "WARN", msg);
}

pub fn error(msg: &str) {
    line(3, "ERROR", msg);
}
