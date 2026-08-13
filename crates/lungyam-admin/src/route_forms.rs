use std::collections::BTreeMap;

use askama::Template;
use lungyam_core::config::{Config, HeaderTransform, RateLimitConfig, RouteConfig, RoutePolicies};
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct RouteForm {
    pub name: String,
    #[serde(default)]
    pub host: String,
    pub path: String,
    #[serde(default)]
    pub methods: String,
    pub upstream: String,
    #[serde(default)]
    pub priority: String,
    #[serde(default)]
    pub request_add_headers: String,
    #[serde(default)]
    pub request_remove_headers: String,
    #[serde(default)]
    pub response_add_headers: String,
    #[serde(default)]
    pub response_remove_headers: String,
    #[serde(default)]
    pub rate_limit_requests: String,
    #[serde(default)]
    pub rate_limit_window_seconds: String,
    #[serde(default)]
    pub max_request_body_bytes: String,
}

#[derive(Template)]
#[template(path = "route-new.html")]
struct RouteFormTemplate {
    overview_active: bool,
    routes_active: bool,
    upstreams: Vec<String>,
}

#[derive(Template)]
#[template(path = "fragments/route-validation.html")]
struct RouteValidationTemplate {
    valid: bool,
    route_name: String,
    message: String,
}

pub(crate) fn render_new_route(config: &Config) -> askama::Result<String> {
    RouteFormTemplate {
        overview_active: false,
        routes_active: true,
        upstreams: config.upstreams.keys().cloned().collect(),
    }
    .render()
}

pub(crate) fn render_validation(config: &Config, form: RouteForm) -> askama::Result<String> {
    let route_name = form.name.trim().to_owned();
    let result = candidate_route(form).and_then(|candidate| {
        let mut candidate_config = config.clone();
        candidate_config.routes.push(candidate);
        candidate_config
            .validate()
            .map_err(|error| error.to_string())
    });

    let (valid, message) = match result {
        Ok(()) => (
            true,
            "Candidate passed the active Lungyam configuration validation rules.".to_owned(),
        ),
        Err(message) => (false, message),
    };

    RouteValidationTemplate {
        valid,
        route_name,
        message,
    }
    .render()
}

fn candidate_route(form: RouteForm) -> Result<RouteConfig, String> {
    let priority = parse_optional(&form.priority, "priority")?.unwrap_or(0);
    let max_request_body_bytes =
        parse_optional(&form.max_request_body_bytes, "max request body bytes")?;
    let request_headers = parse_header_transform(
        &form.request_add_headers,
        &form.request_remove_headers,
        "request",
    )?;
    let response_headers = parse_header_transform(
        &form.response_add_headers,
        &form.response_remove_headers,
        "response",
    )?;

    let rate_requests = parse_optional(&form.rate_limit_requests, "rate-limit requests")?;
    let rate_window = parse_optional(&form.rate_limit_window_seconds, "rate-limit window seconds")?;
    let rate_limit = match (rate_requests, rate_window) {
        (None, None) => None,
        (Some(requests), Some(window_seconds)) => Some(RateLimitConfig {
            requests,
            window_seconds,
        }),
        _ => {
            return Err(
                "rate-limit requests and window seconds must be provided together".to_owned(),
            );
        }
    };

    Ok(RouteConfig {
        name: form.name.trim().to_owned(),
        host: non_empty(form.host),
        path: form.path.trim().to_owned(),
        methods: form
            .methods
            .split(',')
            .map(str::trim)
            .filter(|method| !method.is_empty())
            .map(ToOwned::to_owned)
            .collect(),
        upstream: form.upstream.trim().to_owned(),
        priority,
        policies: RoutePolicies {
            request_headers,
            response_headers,
            rate_limit,
            max_request_body_bytes,
        },
    })
}

fn parse_header_transform(
    add_input: &str,
    remove_input: &str,
    direction: &str,
) -> Result<HeaderTransform, String> {
    let mut add = BTreeMap::new();
    for (index, raw_line) in add_input.lines().enumerate() {
        let line = raw_line.trim();
        if line.is_empty() {
            continue;
        }
        let Some((name, value)) = line.split_once(':') else {
            return Err(format!(
                "{direction} add header line {} must use 'name: value'",
                index + 1
            ));
        };
        let name = name.trim();
        let value = value.trim();
        if name.is_empty() {
            return Err(format!(
                "{direction} add header line {} has an empty name",
                index + 1
            ));
        }
        if add.insert(name.to_owned(), value.to_owned()).is_some() {
            return Err(format!(
                "{direction} add header '{name}' is configured more than once"
            ));
        }
    }

    let remove = remove_input
        .split([',', '\n'])
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(ToOwned::to_owned)
        .collect();

    Ok(HeaderTransform { add, remove })
}

fn non_empty(value: String) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_owned())
}

fn parse_optional<T>(value: &str, label: &str) -> Result<Option<T>, String>
where
    T: std::str::FromStr,
{
    let value = value.trim();
    if value.is_empty() {
        return Ok(None);
    }

    value
        .parse::<T>()
        .map(Some)
        .map_err(|_| format!("{label} must be a valid number"))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use lungyam_core::config::{AdminConfig, ServerConfig, UpstreamConfig};

    use super::{RouteForm, render_validation};

    #[test]
    fn validates_candidate_with_header_transforms_and_core_rules() {
        let html = render_validation(
            &test_config(),
            RouteForm {
                name: "new-route".to_owned(),
                host: "api.test".to_owned(),
                path: "/new".to_owned(),
                methods: "GET, POST".to_owned(),
                upstream: "api".to_owned(),
                priority: "100".to_owned(),
                request_add_headers: "x-added: from-admin\nx-trace: yes".to_owned(),
                request_remove_headers: "x-remove-me".to_owned(),
                response_add_headers: "x-response: enabled".to_owned(),
                response_remove_headers: "server, x-internal".to_owned(),
                rate_limit_requests: "10".to_owned(),
                rate_limit_window_seconds: "60".to_owned(),
                max_request_body_bytes: "1024".to_owned(),
            },
        )
        .expect("validation fragment should render");

        assert!(html.contains("Configuration is valid"));
        assert!(html.contains("new-route"));
    }

    #[test]
    fn surfaces_core_header_validation_errors() {
        let html = render_validation(
            &test_config(),
            RouteForm {
                name: "bad-header".to_owned(),
                host: String::new(),
                path: "/".to_owned(),
                methods: String::new(),
                upstream: "api".to_owned(),
                priority: "0".to_owned(),
                request_add_headers: "bad header: value".to_owned(),
                request_remove_headers: String::new(),
                response_add_headers: String::new(),
                response_remove_headers: String::new(),
                rate_limit_requests: String::new(),
                rate_limit_window_seconds: String::new(),
                max_request_body_bytes: String::new(),
            },
        )
        .expect("validation fragment should render");

        assert!(html.contains("Route is invalid"));
        assert!(html.contains("header add name"));
    }

    #[test]
    fn rejects_malformed_header_line() {
        let html = render_validation(
            &test_config(),
            RouteForm {
                name: "bad-header-line".to_owned(),
                host: String::new(),
                path: "/".to_owned(),
                methods: String::new(),
                upstream: "api".to_owned(),
                priority: "0".to_owned(),
                request_add_headers: "missing-separator".to_owned(),
                request_remove_headers: String::new(),
                response_add_headers: String::new(),
                response_remove_headers: String::new(),
                rate_limit_requests: String::new(),
                rate_limit_window_seconds: String::new(),
                max_request_body_bytes: String::new(),
            },
        )
        .expect("validation fragment should render");

        assert!(html.contains("must use"));
        assert!(html.contains("name: value"));
    }

    #[test]
    fn rejects_incomplete_rate_limit_input() {
        let html = render_validation(
            &test_config(),
            RouteForm {
                name: "bad-rate".to_owned(),
                host: String::new(),
                path: "/".to_owned(),
                methods: String::new(),
                upstream: "api".to_owned(),
                priority: "0".to_owned(),
                request_add_headers: String::new(),
                request_remove_headers: String::new(),
                response_add_headers: String::new(),
                response_remove_headers: String::new(),
                rate_limit_requests: "10".to_owned(),
                rate_limit_window_seconds: String::new(),
                max_request_body_bytes: String::new(),
            },
        )
        .expect("validation fragment should render");

        assert!(html.contains("must be provided together"));
    }

    fn test_config() -> lungyam_core::config::Config {
        let mut upstreams = BTreeMap::new();
        upstreams.insert(
            "api".to_owned(),
            UpstreamConfig {
                endpoints: vec!["127.0.0.1:3000".to_owned()],
                connect_timeout_ms: None,
                read_timeout_ms: None,
                write_timeout_ms: None,
                health_check_interval_seconds: 5,
            },
        );

        lungyam_core::config::Config {
            server: ServerConfig {
                listen: "127.0.0.1:8080".to_owned(),
            },
            admin: AdminConfig {
                enabled: true,
                listen: "127.0.0.1:9090".to_owned(),
                read_only: true,
            },
            upstreams,
            routes: Vec::new(),
        }
    }
}
