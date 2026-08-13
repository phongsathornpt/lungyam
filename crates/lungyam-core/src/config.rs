//! Configuration model and validation for Lungyam.

use std::{collections::BTreeMap, fs, net::SocketAddr, path::Path};

use http::{HeaderName, HeaderValue};
use serde::Deserialize;
use thiserror::Error;

/// Top-level Lungyam configuration.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct Config {
    pub server: ServerConfig,
    #[serde(default)]
    pub admin: AdminConfig,
    pub upstreams: BTreeMap<String, UpstreamConfig>,
    #[serde(default)]
    pub routes: Vec<RouteConfig>,
}

impl Config {
    /// Loads and validates configuration from a YAML file.
    pub fn from_path(path: impl AsRef<Path>) -> Result<Self, ConfigError> {
        let yaml = fs::read_to_string(path)?;
        Self::from_yaml(&yaml)
    }

    /// Parses and validates configuration from YAML.
    pub fn from_yaml(yaml: &str) -> Result<Self, ConfigError> {
        let config: Self = serde_yaml::from_str(yaml)?;
        config.validate()?;
        Ok(config)
    }

    /// Checks references and invariants that YAML deserialization cannot enforce.
    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.server.listen.trim().is_empty() {
            return Err(ConfigError::Validation(
                "server.listen must not be empty".to_owned(),
            ));
        }

        if self.admin.listen.trim().is_empty() {
            return Err(ConfigError::Validation(
                "admin.listen must not be empty".to_owned(),
            ));
        }
        if self.admin.listen.parse::<SocketAddr>().is_err() {
            return Err(ConfigError::Validation(
                "admin.listen must be a valid socket address".to_owned(),
            ));
        }

        if self.upstreams.is_empty() {
            return Err(ConfigError::Validation(
                "at least one upstream is required".to_owned(),
            ));
        }

        for (name, upstream) in &self.upstreams {
            if upstream.endpoints.is_empty() {
                return Err(ConfigError::Validation(format!(
                    "upstream '{name}' must contain at least one endpoint"
                )));
            }
            if upstream
                .endpoints
                .iter()
                .any(|endpoint| endpoint.trim().is_empty())
            {
                return Err(ConfigError::Validation(format!(
                    "upstream '{name}' contains an empty endpoint"
                )));
            }
            if upstream.health_check_interval_seconds == 0 {
                return Err(ConfigError::Validation(format!(
                    "upstream '{name}' health_check_interval_seconds must be greater than zero"
                )));
            }
        }

        let mut route_names = std::collections::BTreeSet::new();
        for route in &self.routes {
            if !route_names.insert(route.name.as_str()) {
                return Err(ConfigError::Validation(format!(
                    "duplicate route name '{}'",
                    route.name
                )));
            }
            if !route.path.starts_with('/') {
                return Err(ConfigError::Validation(format!(
                    "route '{}' path must start with '/'",
                    route.name
                )));
            }
            if !self.upstreams.contains_key(&route.upstream) {
                return Err(ConfigError::Validation(format!(
                    "route '{}' references unknown upstream '{}'",
                    route.name, route.upstream
                )));
            }
            if route.methods.iter().any(|method| method.trim().is_empty()) {
                return Err(ConfigError::Validation(format!(
                    "route '{}' contains an empty HTTP method",
                    route.name
                )));
            }
            validate_header_transform(&route.name, "request", &route.policies.request_headers)?;
            validate_header_transform(&route.name, "response", &route.policies.response_headers)?;
            if let Some(limit) = &route.policies.rate_limit {
                if limit.requests == 0 || limit.window_seconds == 0 {
                    return Err(ConfigError::Validation(format!(
                        "route '{}' rate limit values must be greater than zero",
                        route.name
                    )));
                }
            }
        }

        Ok(())
    }
}

/// Listener settings.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct ServerConfig {
    pub listen: String,
}

/// Admin control-plane listener settings.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct AdminConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_admin_listen")]
    pub listen: String,
}

impl Default for AdminConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            listen: default_admin_listen(),
        }
    }
}

/// A named pool of backend endpoints.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct UpstreamConfig {
    pub endpoints: Vec<String>,
    #[serde(default)]
    pub connect_timeout_ms: Option<u64>,
    #[serde(default)]
    pub read_timeout_ms: Option<u64>,
    #[serde(default)]
    pub write_timeout_ms: Option<u64>,
    #[serde(default = "default_health_check_interval_seconds")]
    pub health_check_interval_seconds: u64,
}

/// Declarative request route.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct RouteConfig {
    pub name: String,
    #[serde(default)]
    pub host: Option<String>,
    #[serde(default = "default_route_path")]
    pub path: String,
    #[serde(default)]
    pub methods: Vec<String>,
    pub upstream: String,
    #[serde(default)]
    pub priority: i32,
    #[serde(default)]
    pub policies: RoutePolicies,
}

/// Policies attached to a route.
#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
pub struct RoutePolicies {
    #[serde(default)]
    pub request_headers: HeaderTransform,
    #[serde(default)]
    pub response_headers: HeaderTransform,
    #[serde(default)]
    pub rate_limit: Option<RateLimitConfig>,
    #[serde(default)]
    pub max_request_body_bytes: Option<usize>,
}

/// Header mutations applied in order: remove, then add/replace.
#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
pub struct HeaderTransform {
    #[serde(default)]
    pub add: BTreeMap<String, String>,
    #[serde(default)]
    pub remove: Vec<String>,
}

/// Fixed-window local rate limit configuration.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct RateLimitConfig {
    pub requests: u64,
    pub window_seconds: u64,
}

/// Configuration loading and validation errors.
#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("failed to read configuration: {0}")]
    Io(#[from] std::io::Error),
    #[error("failed to parse YAML configuration: {0}")]
    Parse(#[from] serde_yaml::Error),
    #[error("invalid configuration: {0}")]
    Validation(String),
}

fn validate_header_transform(
    route_name: &str,
    direction: &str,
    transform: &HeaderTransform,
) -> Result<(), ConfigError> {
    for name in &transform.remove {
        if HeaderName::from_bytes(name.as_bytes()).is_err() {
            return Err(ConfigError::Validation(format!(
                "route '{route_name}' {direction} header remove name '{name}' is invalid"
            )));
        }
    }

    for (name, value) in &transform.add {
        if HeaderName::from_bytes(name.as_bytes()).is_err() {
            return Err(ConfigError::Validation(format!(
                "route '{route_name}' {direction} header add name '{name}' is invalid"
            )));
        }
        if value.parse::<HeaderValue>().is_err() {
            return Err(ConfigError::Validation(format!(
                "route '{route_name}' {direction} header value for '{name}' is invalid"
            )));
        }
    }

    Ok(())
}

fn default_route_path() -> String {
    "/".to_owned()
}

fn default_admin_listen() -> String {
    "127.0.0.1:9090".to_owned()
}

const fn default_health_check_interval_seconds() -> u64 {
    5
}

#[cfg(test)]
mod tests {
    use super::{Config, ConfigError};

    const VALID: &str = r#"
server:
  listen: 0.0.0.0:8080
upstreams:
  api:
    endpoints:
      - 127.0.0.1:3000
routes:
  - name: api
    path: /api
    methods: [GET, POST]
    upstream: api
"#;

    #[test]
    fn parses_valid_config() {
        let config = Config::from_yaml(VALID).expect("valid configuration");
        assert_eq!(config.server.listen, "0.0.0.0:8080");
        assert!(!config.admin.enabled);
        assert_eq!(config.admin.listen, "127.0.0.1:9090");
        assert_eq!(config.routes[0].upstream, "api");
        assert_eq!(config.upstreams["api"].health_check_interval_seconds, 5);
    }

    #[test]
    fn accepts_enabled_admin_listener() {
        let yaml = VALID.replace(
            "upstreams:",
            "admin:\n  enabled: true\n  listen: 127.0.0.1:9091\nupstreams:",
        );
        let config = Config::from_yaml(&yaml).expect("valid admin configuration");
        assert!(config.admin.enabled);
        assert_eq!(config.admin.listen, "127.0.0.1:9091");
    }

    #[test]
    fn rejects_invalid_admin_listener() {
        let yaml = VALID.replace(
            "upstreams:",
            "admin:\n  enabled: true\n  listen: not-a-socket\nupstreams:",
        );
        let error = Config::from_yaml(&yaml).expect_err("invalid admin listener");
        assert!(matches!(error, ConfigError::Validation(_)));
    }

    #[test]
    fn rejects_unknown_upstream() {
        let invalid = VALID.replace("upstream: api", "upstream: missing");
        let error = Config::from_yaml(&invalid).expect_err("invalid upstream reference");
        assert!(matches!(error, ConfigError::Validation(_)));
    }

    #[test]
    fn rejects_duplicate_route_names() {
        let invalid = format!("{VALID}\n  - name: api\n    path: /other\n    upstream: api\n");
        let error = Config::from_yaml(&invalid).expect_err("duplicate route name");
        assert!(matches!(error, ConfigError::Validation(_)));
    }

    #[test]
    fn rejects_zero_health_check_interval() {
        let invalid = VALID.replace(
            "endpoints:\n      - 127.0.0.1:3000",
            "endpoints:\n      - 127.0.0.1:3000\n    health_check_interval_seconds: 0",
        );
        let error = Config::from_yaml(&invalid).expect_err("invalid health check interval");
        assert!(matches!(error, ConfigError::Validation(_)));
    }

    #[test]
    fn rejects_invalid_header_transform_name() {
        let invalid = VALID.replace(
            "    upstream: api",
            "    upstream: api\n    policies:\n      request_headers:\n        add:\n          'bad header': value",
        );
        let error = Config::from_yaml(&invalid).expect_err("invalid header name");
        assert!(matches!(error, ConfigError::Validation(_)));
        assert!(error.to_string().contains("header add name"));
    }

    #[test]
    fn rejects_invalid_header_transform_value() {
        let invalid = VALID.replace(
            "    upstream: api",
            "    upstream: api\n    policies:\n      response_headers:\n        add:\n          x-test: \"bad\\nvalue\"",
        );
        let error = Config::from_yaml(&invalid).expect_err("invalid header value");
        assert!(matches!(error, ConfigError::Validation(_)));
        assert!(error.to_string().contains("header value"));
    }
}
