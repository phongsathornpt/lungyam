use std::collections::BTreeMap;

use crate::config::{Config, RouteConfig};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ConfigDiff {
    pub server_changed: bool,
    pub admin_changed: bool,
    pub upstreams_added: Vec<String>,
    pub upstreams_removed: Vec<String>,
    pub upstreams_changed: Vec<String>,
    pub routes_added: Vec<String>,
    pub routes_removed: Vec<String>,
    pub routes_changed: Vec<String>,
}

impl ConfigDiff {
    #[must_use]
    pub fn between(current: &Config, candidate: &Config) -> Self {
        let current_routes = routes_by_name(&current.routes);
        let candidate_routes = routes_by_name(&candidate.routes);

        Self {
            server_changed: current.server != candidate.server,
            admin_changed: current.admin != candidate.admin,
            upstreams_added: added_keys(&current.upstreams, &candidate.upstreams),
            upstreams_removed: added_keys(&candidate.upstreams, &current.upstreams),
            upstreams_changed: changed_keys(&current.upstreams, &candidate.upstreams),
            routes_added: added_keys(&current_routes, &candidate_routes),
            routes_removed: added_keys(&candidate_routes, &current_routes),
            routes_changed: changed_keys(&current_routes, &candidate_routes),
        }
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        !self.server_changed
            && !self.admin_changed
            && self.upstreams_added.is_empty()
            && self.upstreams_removed.is_empty()
            && self.upstreams_changed.is_empty()
            && self.routes_added.is_empty()
            && self.routes_removed.is_empty()
            && self.routes_changed.is_empty()
    }

    #[must_use]
    pub fn restart_required(&self) -> bool {
        self.server_changed
            || self.admin_changed
            || !self.upstreams_added.is_empty()
            || !self.upstreams_removed.is_empty()
            || !self.upstreams_changed.is_empty()
    }
}

fn routes_by_name(routes: &[RouteConfig]) -> BTreeMap<String, RouteConfig> {
    routes
        .iter()
        .map(|route| (route.name.clone(), route.clone()))
        .collect()
}

fn added_keys<T>(current: &BTreeMap<String, T>, candidate: &BTreeMap<String, T>) -> Vec<String> {
    candidate
        .keys()
        .filter(|name| !current.contains_key(*name))
        .cloned()
        .collect()
}

fn changed_keys<T: PartialEq>(
    current: &BTreeMap<String, T>,
    candidate: &BTreeMap<String, T>,
) -> Vec<String> {
    current
        .iter()
        .filter_map(|(name, value)| {
            candidate
                .get(name)
                .filter(|candidate_value| *candidate_value != value)
                .map(|_| name.clone())
        })
        .collect()
}
