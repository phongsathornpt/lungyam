use std::{
    collections::BTreeMap,
    sync::{Arc, RwLock},
    time::Instant,
};

use thiserror::Error;

use crate::{
    config::{Config, RouteConfig},
    routing::sort_routes,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActiveConfigSummary {
    pub proxy_listen: String,
    pub admin_listen: String,
    pub admin_read_only: bool,
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

#[derive(Debug, Error, PartialEq, Eq)]
pub enum RuntimeConfigApplyError {
    #[error("candidate runtime config is invalid: {0}")]
    InvalidConfig(String),
    #[error("runtime route reload does not support server, admin, or upstream changes")]
    StructuralChange,
}

/// Immutable handle to the configuration currently owned by the runtime.
///
/// Cloning this value is cheap and keeps the underlying configuration alive
/// while a newer snapshot is installed for subsequent requests.
#[derive(Clone, Debug)]
pub struct RuntimeConfigSnapshot {
    config: Arc<Config>,
    routes: Arc<Vec<RouteConfig>>,
}

impl RuntimeConfigSnapshot {
    #[must_use]
    pub fn from_config(config: &Config) -> Self {
        let mut routes = config.routes.clone();
        sort_routes(&mut routes);

        Self {
            config: Arc::new(config.clone()),
            routes: Arc::new(routes),
        }
    }

    #[must_use]
    pub fn config(&self) -> &Config {
        self.config.as_ref()
    }

    /// Returns the immutable route table in proxy evaluation order.
    #[must_use]
    pub fn routes(&self) -> &[RouteConfig] {
        self.routes.as_ref()
    }
}

/// Shared runtime state written by the data plane and read by control-plane adapters.
#[derive(Debug)]
pub struct RuntimeStatus {
    started_at: Instant,
    config: RwLock<RuntimeConfigSnapshot>,
    endpoint_health: RwLock<BTreeMap<(String, String), bool>>,
}

impl RuntimeStatus {
    #[must_use]
    pub fn from_config(config: &Config) -> Self {
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
            config: RwLock::new(RuntimeConfigSnapshot::from_config(config)),
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

    /// Replaces route matching and route policy configuration for subsequent requests.
    ///
    /// Listener and upstream changes remain restart-required because Pingora services,
    /// load balancers, and health-check workers are built during process startup.
    pub fn apply_route_config(&self, candidate: &Config) -> Result<(), RuntimeConfigApplyError> {
        candidate
            .validate()
            .map_err(|error| RuntimeConfigApplyError::InvalidConfig(error.to_string()))?;

        let current = self.config_snapshot();
        if candidate.server != current.config().server
            || candidate.admin != current.config().admin
            || candidate.upstreams != current.config().upstreams
        {
            return Err(RuntimeConfigApplyError::StructuralChange);
        }

        let mut active = self
            .config
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *active = RuntimeConfigSnapshot::from_config(candidate);
        Ok(())
    }

    /// Returns a cheap immutable handle to the currently active runtime configuration.
    #[must_use]
    pub fn config_snapshot(&self) -> RuntimeConfigSnapshot {
        self.config
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    /// Returns the currently active configuration snapshot as an owned value.
    #[must_use]
    pub fn config(&self) -> Config {
        self.config_snapshot().config().clone()
    }

    /// Returns the currently active route configuration snapshot in proxy evaluation order.
    #[must_use]
    pub fn routes(&self) -> Vec<RouteConfig> {
        self.config_snapshot().routes().to_vec()
    }

    #[must_use]
    pub fn snapshot(&self) -> RuntimeSnapshot {
        let config = self.config_snapshot();
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
            active_config: active_config_summary(config.config()),
            endpoint_health,
        }
    }
}

fn active_config_summary(config: &Config) -> ActiveConfigSummary {
    ActiveConfigSummary {
        proxy_listen: config.server.listen.clone(),
        admin_listen: config.admin.listen.clone(),
        admin_read_only: config.admin.read_only,
        route_count: config.routes.len(),
        upstream_count: config.upstreams.len(),
        endpoint_count: config
            .upstreams
            .values()
            .map(|upstream| upstream.endpoints.len())
            .sum(),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use crate::config::{
        AdminConfig, Config, RouteConfig, RoutePolicies, ServerConfig, UpstreamConfig,
    };

    use super::{RuntimeConfigApplyError, RuntimeStatus};

    #[test]
    fn captures_config_and_tracks_endpoint_health() {
        let config = test_config();
        let status = RuntimeStatus::from_config(&config);

        let initial = status.snapshot();
        assert!(initial.active_config.admin_read_only);
        assert_eq!(initial.active_config.route_count, 1);
        assert_eq!(initial.active_config.upstream_count, 1);
        assert_eq!(initial.active_config.endpoint_count, 2);
        assert!(
            initial
                .endpoint_health
                .iter()
                .all(|endpoint| endpoint.healthy)
        );

        let active_config = status.config();
        assert_eq!(active_config.routes.len(), 1);
        assert_eq!(active_config.upstreams.len(), 1);

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

    #[test]
    fn config_snapshot_is_immutable_and_independent_from_source_config() {
        let mut source = test_config();
        let status = RuntimeStatus::from_config(&source);
        let first = status.config_snapshot();
        let second = first.clone();

        source.routes.clear();
        source.upstreams.clear();

        assert_eq!(first.config().routes.len(), 1);
        assert_eq!(first.config().upstreams.len(), 1);
        assert!(std::ptr::eq(first.config(), second.config()));
        assert_eq!(second.routes()[0].name, "api");
    }

    #[test]
    fn config_snapshot_precomputes_proxy_route_order() {
        let mut config = test_config();
        let mut lower = config.routes[0].clone();
        lower.name = "lower".to_owned();
        lower.priority = 10;
        lower.path = "/api/long".to_owned();

        let mut higher = lower.clone();
        higher.name = "higher".to_owned();
        higher.priority = 20;
        higher.path = "/".to_owned();

        let mut specific = higher.clone();
        specific.name = "specific".to_owned();
        specific.path = "/api".to_owned();

        config.routes = vec![lower, higher, specific];
        let status = RuntimeStatus::from_config(&config);
        let snapshot = status.config_snapshot();

        assert_eq!(snapshot.routes()[0].name, "specific");
        assert_eq!(snapshot.routes()[1].name, "higher");
        assert_eq!(snapshot.routes()[2].name, "lower");
        assert_eq!(snapshot.config().routes[0].name, "lower");
    }

    #[test]
    fn route_config_apply_swaps_future_snapshots_and_preserves_pinned_readers() {
        let config = test_config();
        let status = RuntimeStatus::from_config(&config);
        let pinned = status.config_snapshot();

        let mut candidate = config.clone();
        let mut second = candidate.routes[0].clone();
        second.name = "api-v2".to_owned();
        second.path = "/v2".to_owned();
        second.priority = 100;
        candidate.routes.push(second);

        status
            .apply_route_config(&candidate)
            .expect("route-only config should hot apply");

        assert_eq!(pinned.routes().len(), 1);
        assert_eq!(status.config_snapshot().routes().len(), 2);
        assert_eq!(status.config_snapshot().routes()[0].name, "api-v2");
        assert_eq!(status.snapshot().active_config.route_count, 2);
    }

    #[test]
    fn route_config_apply_rejects_structural_changes_without_replacing_active_snapshot() {
        let config = test_config();
        let status = RuntimeStatus::from_config(&config);
        let mut candidate = config.clone();
        candidate
            .upstreams
            .get_mut("api")
            .expect("api upstream")
            .endpoints
            .push("127.0.0.1:3002".to_owned());

        assert_eq!(
            status.apply_route_config(&candidate),
            Err(RuntimeConfigApplyError::StructuralChange)
        );
        assert_eq!(status.config(), config);
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
                read_only: true,
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
