//! `/etc/hostdock-probe/config.toml`.

use serde::Deserialize;
use std::fs;

pub const DEFAULT_PATH: &str = "/etc/hostdock-probe/config.toml";

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(default)]
    pub token: Option<String>,
    #[serde(default)]
    pub collect: Collect,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Collect {
    pub temperature: bool,
    /// Mount points not to report: exact paths, or a prefix ending in `*`.
    pub exclude_mounts: Vec<String>,
    /// Network interfaces not to count: exact names, or a prefix ending in `*`.
    pub exclude_interfaces: Vec<String>,
}

impl Default for Collect {
    fn default() -> Self {
        Collect {
            temperature: true,
            exclude_mounts: Vec::new(),
            exclude_interfaces: Vec::new(),
        }
    }
}

/// `hdp_` followed by 16 to 128 letters and digits.
pub fn valid_token(token: &str) -> bool {
    token.strip_prefix("hdp_").is_some_and(|rest| {
        (16..=128).contains(&rest.len()) && rest.bytes().all(|b| b.is_ascii_alphanumeric())
    })
}

pub fn parse(text: &str) -> Result<Config, String> {
    toml::from_str(text).map_err(|e| e.message().to_string())
}

/// Load the config file (missing is fine when the token comes from the
/// environment), then let `HOSTDOCK_PROBE_TOKEN` override the token.
pub fn load(path: &str) -> Result<Config, String> {
    let mut config = match fs::read_to_string(path) {
        Ok(text) => parse(&text).map_err(|e| format!("{path}: {e}"))?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Config {
            token: None,
            collect: Collect::default(),
        },
        Err(e) => return Err(format!("cannot read {path}: {e}")),
    };
    if let Ok(token) = std::env::var("HOSTDOCK_PROBE_TOKEN")
        && !token.is_empty()
    {
        config.token = Some(token);
    }
    Ok(config)
}

impl Config {
    /// The token, checked for shape. The message never contains the token.
    pub fn token(&self) -> Result<String, String> {
        match self.token.as_deref().map(str::trim) {
            None | Some("") => Err("no token: set `token` in the config file or HOSTDOCK_PROBE_TOKEN".into()),
            Some(t) if !valid_token(t) => Err("the token does not look right (it should start with `hdp_`); copy it again from the HostDock app".into()),
            Some(t) => Ok(t.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_config() {
        let c = parse(
            r#"
token = "hdp_7Kx2mQ9vR4tL8wNc3ZpB6yFd"
[collect]
temperature = false
exclude_mounts = ["/mnt/*"]
exclude_interfaces = ["eth1"]
"#,
        )
        .unwrap();
        assert_eq!(c.token().unwrap(), "hdp_7Kx2mQ9vR4tL8wNc3ZpB6yFd");
        assert!(!c.collect.temperature);
        assert_eq!(c.collect.exclude_mounts, ["/mnt/*"]);
    }

    #[test]
    fn defaults() {
        let c = parse(r#"token = "hdp_7Kx2mQ9vR4tL8wNc3ZpB6yFd""#).unwrap();
        assert!(c.collect.temperature);
        assert!(c.collect.exclude_mounts.is_empty());
    }

    #[test]
    fn typos_are_errors() {
        assert!(parse("tokn = \"x\"").is_err());
        assert!(parse("[collect]\ntemprature = true").is_err());
    }

    #[test]
    fn token_shape() {
        assert!(valid_token("hdp_7Kx2mQ9vR4tL8wNc3ZpB6yFd"));
        assert!(!valid_token("7Kx2mQ9vR4tL8wNc3ZpB6yFd"));
        assert!(!valid_token("hdp_short"));
        assert!(!valid_token("hdp_has-dash-in-it-xxxxxxxx"));
    }

    #[test]
    fn errors_never_contain_the_token() {
        let c = parse(r#"token = "hdp_secret-but-malformed-value""#).unwrap();
        let err = c.token().unwrap_err();
        assert!(!err.contains("secret"));
    }
}
