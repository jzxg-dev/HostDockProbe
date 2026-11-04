//! Load averages from `/proc/loadavg`, uptime from `/proc/uptime`, and the
//! process count from the names of `/proc` entries.

use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Load {
    pub one: f64,
    pub five: f64,
    pub fifteen: f64,
}

pub fn parse_loadavg(text: &str) -> Option<Load> {
    let mut f = text.split_ascii_whitespace();
    Some(Load {
        one: f.next()?.parse().ok()?,
        five: f.next()?.parse().ok()?,
        fifteen: f.next()?.parse().ok()?,
    })
}

/// Seconds since boot.
pub fn parse_uptime(text: &str) -> Option<f64> {
    let v: f64 = text.split_ascii_whitespace().next()?.parse().ok()?;
    v.is_finite().then_some(v)
}

/// A process is a directory in `/proc` whose name is all digits.
/// (The second number in `/proc/loadavg` counts threads, not processes.)
pub fn is_pid(name: &str) -> bool {
    !name.is_empty() && name.bytes().all(|b| b.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_real_loadavg() {
        let l = parse_loadavg(include_str!("../../fixtures/debian_12/loadavg")).unwrap();
        assert_eq!(
            l,
            Load {
                one: 0.24,
                five: 0.20,
                fifteen: 0.13
            }
        );
    }

    #[test]
    fn parses_uptime() {
        assert_eq!(parse_uptime("3643200.51 1200.00\n"), Some(3643200.51));
        assert!(parse_uptime(include_str!("../../fixtures/alpine_3_20/uptime")).is_some());
        assert_eq!(parse_uptime(""), None);
    }

    #[test]
    fn recognises_pids() {
        assert!(is_pid("1"));
        assert!(is_pid("40213"));
        assert!(!is_pid("self"));
        assert!(!is_pid("thread-self"));
        assert!(!is_pid(""));
    }
}
