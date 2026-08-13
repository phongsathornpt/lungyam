use askama::Template;
use lungyam_core::{config::Config, routing::find_matching_route};
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct RouteMatchForm {
    #[serde(default)]
    pub host: String,
    pub path: String,
    pub method: String,
}

#[derive(Template)]
#[template(path = "fragments/route-simulation.html")]
struct RouteSimulationTemplate {
    valid: bool,
    matched: bool,
    route_name: String,
    upstream: String,
    priority: i32,
    message: String,
}

pub(crate) fn render_simulation(config: &Config, form: RouteMatchForm) -> askama::Result<String> {
    let host = form.host.trim();
    let path = form.path.trim();
    let method = form.method.trim();

    let validation_error = if path.is_empty() {
        Some("path is required")
    } else if !path.starts_with('/') {
        Some("path must start with '/'")
    } else if method.is_empty() {
        Some("method is required")
    } else {
        None
    };

    let template = if let Some(message) = validation_error {
        RouteSimulationTemplate {
            valid: false,
            matched: false,
            route_name: String::new(),
            upstream: String::new(),
            priority: 0,
            message: message.to_owned(),
        }
    } else if let Some(route) = find_matching_route(
        &config.routes,
        (!host.is_empty()).then_some(host),
        path,
        method,
    ) {
        RouteSimulationTemplate {
            valid: true,
            matched: true,
            route_name: route.name.clone(),
            upstream: route.upstream.clone(),
            priority: route.priority,
            message: format!("{method} {path} matches the active route evaluation rules."),
        }
    } else {
        RouteSimulationTemplate {
            valid: true,
            matched: false,
            route_name: String::new(),
            upstream: String::new(),
            priority: 0,
            message: format!("No active route matches {method} {path}."),
        }
    };

    template.render()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use lungyam_core::config::{
        AdminConfig, Config, RouteConfig, RoutePolicies, ServerConfig, UpstreamConfig,
    };

    use super::{RouteMatchForm, render_simulation};

    #[test]
    fn matches_using_shared_data_plane_semantics() {
        let html = render_simulation(
            &test_config(),
            RouteMatchForm {
                host: "api.test:8443".to_owned(),
                path: "/echo/users".to_owned(),
                method: "post".to_owned(),
            },
        )
        .expect("simulation should render");

        assert!(html.contains("Matched route"));
        assert!(html.contains("echo"));
        assert!(html.contains("fixture"));
    }

    #[test]
    fn reports_no_match() {
        let html = render_simulation(
            &test_config(),
            RouteMatchForm {
                host: "api.test".to_owned(),
                path: "/echo".to_owned(),
                method: "GET".to_owned(),
            },
        )
        .expect("simulation should render");

        assert!(html.contains("No route matched"));
    }

    #[test]
    fn rejects_invalid_simulation_input() {
        let html = render_simulation(
            &test_config(),
            RouteMatchForm {
                host: String::new(),
                path: "echo".to_owned(),
                method: "POST".to_owned(),
            },
        )
        .expect("simulation should render");

        assert!(html.contains("Simulation input is invalid"));
        assert!(html.contains("path must start"));
    }

    fn test_config() -> Config {
        let mut upstreams = BTreeMap::new();
        upstreams.insert(
            "fixture".to_owned(),
            UpstreamConfig {
                endpoints: vec!["127.0.0.1:3001".to_owned()],
                connect_timeout_ms: None,
                read_timeout_ms: None,
                write_timeout_ms: None,
                health_check_interval_seconds: 5,
            },
        );

        Config {
            server: ServerConfig {
                listen: "127.0.0.1:18080".to_owned(),
            },
            admin: AdminConfig {
                enabled: true,
                listen: "127.0.0.1:19090".to_owned(),
            },
            upstreams,
            routes: vec![RouteConfig {
                name: "echo".to_owned(),
                host: Some("api.test".to_owned()),
                path: "/echo".to_owned(),
                methods: vec!["POST".to_owned()],
                upstream: "fixture".to_owned(),
                priority: 100,
                policies: RoutePolicies::default(),
            }],
        }
    }
}
