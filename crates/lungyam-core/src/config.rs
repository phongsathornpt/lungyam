//! Configuration model and validation for Lungyam.

use std::{collections::BTreeMap, fs, path::Path};

use serde::Deserialize;
use thiserror::Error;

/// Top-level Lungyam configuration.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct Config {
    pub server: ServerConfig,
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
        }

        Ok(())
    }
}

/// Listener settings.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct ServerConfig {
    pub listen: String,
}

/// A named pool of backend endpoints.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct UpstreamConfig {
    pub endpoints: Vec<String>,
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

fn default_route_path() -> String {
    "/".to_owned()
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
        assert_eq!(config.routes[0].upstream, "api");
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
}
