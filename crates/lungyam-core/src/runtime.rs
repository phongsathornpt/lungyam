use std::{collections::BTreeMap, sync::RwLock, time::Instant};

use crate::config::{Config, RouteConfig};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActiveConfigSummary {
    pub proxy_listen: String,
    pub admin_listen: String,
    pub route_count: usize,
    pub upstream_count: usize,
    pub endpoint_count: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EndpointHealth {
    pub upstream: String,
    pub endpoint: String,
    pub healthy: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuntimeSnapshot {
    pub uptime_seconds: u64,
    pub active_config: ActiveConfigSummary,
    pub endpoint_health: Vec<EndpointHealth>,
}

/// Shared runtime state written by the data plane and read by control-plane adapters.
#[derive(Debug)]
pub struct RuntimeStatus {
    started_at: Instant,
    active_config: ActiveConfigSummary,
    route_configs: Vec<RouteConfig>,
    endpoint_health: RwLock<BTreeMap<(String, String), bool>>,
}

impl RuntimeStatus {
    #[must_use]
    pub fn from_config(config: &Config) -> Self {
        let endpoint_count = config
            .upstreams
            .values()
            .map(|upstream| upstream.endpoints.len())
            .sum();
        let endpoint_health = config
            .upstreams
            .iter()
            .flat_map(|(upstream_name, upstream)| {
                upstream
                    .endpoints
                    .iter()
                    .map(|endpoint| ((upstream_name.clone(), endpoint.clone()), true))
            })
            .collect();

        Self {
            started_at: Instant::now(),
            active_config: ActiveConfigSummary {
                proxy_listen: config.server.listen.clone(),
                admin_listen: config.admin.listen.clone(),
                route_count: config.routes.len(),
                upstream_count: config.upstreams.len(),
                endpoint_count,
            },
            route_configs: config.routes.clone(),
            endpoint_health: RwLock::new(endpoint_health),
        }
    }

    pub fn set_endpoint_health(&self, upstream: &str, endpoint: &str, healthy: bool) {
        let mut states = self
            .endpoint_health
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        states.insert((upstream.to_owned(), endpoint.to_owned()), healthy);
    }

    /// Returns the currently active route configuration snapshot.
    #[must_use]
    pub fn routes(&self) -> Vec<RouteConfig> {
        self.route_configs.clone()
    }

    #[must_use]
    pub fn snapshot(&self) -> RuntimeSnapshot {
        let endpoint_health = self
            .endpoint_health
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .iter()
            .map(|((upstream, endpoint), healthy)| EndpointHealth {
                upstream: upstream.clone(),
                endpoint: endpoint.clone(),
                healthy: *healthy,
            })
            .collect();

        RuntimeSnapshot {
            uptime_seconds: self.started_at.elapsed().as_secs(),
            active_config: self.active_config.clone(),
            endpoint_health,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use crate::config::{
        AdminConfig, Config, RouteConfig, RoutePolicies, ServerConfig, UpstreamConfig,
    };

    use super::RuntimeStatus;

    #[test]
    fn captures_config_and_tracks_endpoint_health() {
        let config = test_config();
        let status = RuntimeStatus::from_config(&config);

        let initial = status.snapshot();
        assert_eq!(initial.active_config.route_count, 1);
        assert_eq!(initial.active_config.upstream_count, 1);
        assert_eq!(initial.active_config.endpoint_count, 2);
        assert!(
            initial
                .endpoint_health
                .iter()
                .all(|endpoint| endpoint.healthy)
        );

        let routes = status.routes();
        assert_eq!(routes.len(), 1);
        assert_eq!(routes[0].name, "api");
        assert_eq!(routes[0].upstream, "api");

        status.set_endpoint_health("api", "127.0.0.1:3001", false);
        let updated = status.snapshot();
        let failed = updated
            .endpoint_health
            .iter()
            .find(|endpoint| endpoint.endpoint == "127.0.0.1:3001")
            .expect("configured endpoint should exist");
        assert!(!failed.healthy);
    }

    fn test_config() -> Config {
        let mut upstreams = BTreeMap::new();
        upstreams.insert(
            "api".to_owned(),
            UpstreamConfig {
                endpoints: vec!["127.0.0.1:3000".to_owned(), "127.0.0.1:3001".to_owned()],
                connect_timeout_ms: None,
                read_timeout_ms: None,
                write_timeout_ms: None,
                health_check_interval_seconds: 5,
            },
        );

        Config {
            server: ServerConfig {
                listen: "0.0.0.0:8080".to_owned(),
            },
            admin: AdminConfig {
                enabled: true,
                listen: "127.0.0.1:9090".to_owned(),
            },
            upstreams,
            routes: vec![RouteConfig {
                name: "api".to_owned(),
                host: None,
                path: "/".to_owned(),
                methods: Vec::new(),
                upstream: "api".to_owned(),
                priority: 0,
                policies: RoutePolicies::default(),
            }],
        }
    }
}
