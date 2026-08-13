use askama::Template;
use axum::{
    Form,
    extract::State,
    http::StatusCode,
    response::{Html, IntoResponse, Response},
};
use lungyam_core::{
    config::{Config, UpstreamConfig},
    config_diff::ConfigDiff,
    lifecycle::FileConfigLifecycle,
};
use serde::Deserialize;

use crate::{
    AdminState,
    route_forms::{self, RouteForm},
    security,
};

#[derive(Debug, Deserialize)]
pub(super) struct StageRouteForm {
    #[serde(default)]
    operation: String,
    #[serde(default)]
    original_name: String,
    #[serde(flatten)]
    route: RouteForm,
    #[serde(default)]
    upstream_original_name: String,
    #[serde(default)]
    upstream_name: String,
    #[serde(default)]
    endpoints: String,
    #[serde(default)]
    connect_timeout: String,
    #[serde(default)]
    read_timeout: String,
    #[serde(default)]
    write_timeout: String,
    #[serde(default)]
    health_check_interval: String,
}

#[derive(Template)]
#[template(path = "fragments/route-stage.html")]
struct RouteStageTemplate {
    success: bool,
    route_name: String,
    revision: String,
    message: String,
}

#[derive(Template)]
#[template(path = "fragments/upstream-validation.html")]
struct UpstreamValidationTemplate {
    valid: bool,
    upstream_name: String,
    restart_required: bool,
    message: String,
}

pub(super) async fn stage_route(
    State(state): State<AdminState>,
    Form(form): Form<StageRouteForm>,
) -> Response {
    let config = state.runtime.config();
    let operation = form.operation.trim();

    if operation == "validate-upstream" {
        return render_upstream_validation(&config, &form);
    }

    let subject_name = if operation == "stage-upstream" {
        form.upstream_name.trim().to_owned()
    } else {
        form.route.name.trim().to_owned()
    };

    if config.admin.read_only {
        return render_write_denied(operation, subject_name, "Admin is configured as read-only.");
    }
    if !security::writes_enabled(&config) {
        return render_write_denied(
            operation,
            subject_name,
            "Admin writes require a loopback listener until authentication is configured.",
        );
    }
    if !security::csrf_token().verify(&form.route.csrf_token) {
        return render_write_denied(operation, subject_name, "CSRF token validation failed.");
    }

    let Some(config_path) = state.config_path.as_ref() else {
        return render_write_denied(
            operation,
            subject_name,
            "Config path is unavailable; staging is disabled for this admin router.",
        );
    };

    if operation == "stage-upstream" {
        let candidate = match candidate_upstream_config(&config, &form) {
            Ok(candidate) => candidate,
            Err(message) => {
                return render_upstream_stage(
                    StatusCode::UNPROCESSABLE_ENTITY,
                    false,
                    form.upstream_name.trim().to_owned(),
                    String::new(),
                    message,
                );
            }
        };
        let upstream_name = form.upstream_name.trim().to_owned();
        let reason = if form.upstream_original_name.trim().is_empty() {
            format!("stage upstream create '{upstream_name}'")
        } else {
            format!("stage upstream update '{upstream_name}'")
        };
        return match FileConfigLifecycle::new(config_path).stage(
            &candidate,
            Some("admin-web".to_owned()),
            Some(reason),
        ) {
            Ok(metadata) => render_upstream_stage(
                StatusCode::OK,
                true,
                upstream_name,
                format!("#{:06}", metadata.revision),
                String::new(),
            ),
            Err(error) => render_upstream_stage(
                StatusCode::INTERNAL_SERVER_ERROR,
                false,
                upstream_name,
                String::new(),
                error.to_string(),
            ),
        };
    }

    let route_name = form.route.name.trim().to_owned();
    let candidate = match operation {
        "" | "create" => route_forms::candidate_config(&config, form.route),
        "update" => candidate_updated_config(&config, &form.original_name, form.route),
        other => Err(format!("unsupported route mutation operation '{other}'")),
    };
    let candidate = match candidate {
        Ok(candidate) => candidate,
        Err(message) => {
            return render_route_stage(
                StatusCode::UNPROCESSABLE_ENTITY,
                false,
                route_name,
                String::new(),
                message,
            );
        }
    };

    let reason = if operation == "update" {
        format!(
            "stage route update '{}' -> '{route_name}'",
            form.original_name.trim()
        )
    } else {
        format!("stage route '{route_name}'")
    };

    match FileConfigLifecycle::new(config_path).stage(
        &candidate,
        Some("admin-web".to_owned()),
        Some(reason),
    ) {
        Ok(metadata) => render_route_stage(
            StatusCode::OK,
            true,
            route_name,
            format!("#{:06}", metadata.revision),
            String::new(),
        ),
        Err(error) => render_route_stage(
            StatusCode::INTERNAL_SERVER_ERROR,
            false,
            route_name,
            String::new(),
            error.to_string(),
        ),
    }
}

fn render_upstream_validation(config: &Config, form: &StageRouteForm) -> Response {
    let upstream_name = form.upstream_name.trim().to_owned();
    let (valid, restart_required, message) = match candidate_upstream_config(config, form) {
        Ok(candidate) => {
            let diff = ConfigDiff::between(config, &candidate);
            (
                true,
                diff.restart_required(),
                "Candidate passed the active Lungyam configuration validation rules.".to_owned(),
            )
        }
        Err(message) => (false, false, message),
    };

    match (UpstreamValidationTemplate {
        valid,
        upstream_name,
        restart_required,
        message,
    })
    .render()
    {
        Ok(html) => Html(html).into_response(),
        Err(error) => {
            log::error!("failed to render upstream validation: {error}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

fn candidate_upstream_config(config: &Config, form: &StageRouteForm) -> Result<Config, String> {
    let upstream_name = form.upstream_name.trim();
    if upstream_name.is_empty() {
        return Err("upstream name must not be empty".to_owned());
    }

    let original_name = form.upstream_original_name.trim();
    if original_name.is_empty() {
        if config.upstreams.contains_key(upstream_name) {
            return Err(format!("upstream '{upstream_name}' already exists"));
        }
    } else {
        if original_name != upstream_name {
            return Err("renaming an existing upstream is not supported yet".to_owned());
        }
        if !config.upstreams.contains_key(original_name) {
            return Err(format!("upstream '{original_name}' was not found"));
        }
    }

    let endpoints = form
        .endpoints
        .split([',', '\n'])
        .map(str::trim)
        .filter(|endpoint| !endpoint.is_empty())
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    if endpoints.is_empty() {
        return Err("at least one upstream endpoint is required".to_owned());
    }

    let health_check_interval_seconds =
        parse_required_duration(&form.health_check_interval, "health-check interval", "s")?;
    let mut candidate = config.clone();
    candidate.upstreams.insert(
        upstream_name.to_owned(),
        UpstreamConfig {
            endpoints,
            connect_timeout_ms: parse_optional_duration(
                &form.connect_timeout,
                "connect timeout",
                "ms",
            )?,
            read_timeout_ms: parse_optional_duration(&form.read_timeout, "read timeout", "ms")?,
            write_timeout_ms: parse_optional_duration(&form.write_timeout, "write timeout", "ms")?,
            health_check_interval_seconds,
        },
    );
    candidate.validate().map_err(|error| error.to_string())?;
    Ok(candidate)
}

fn parse_optional_duration(value: &str, label: &str, suffix: &str) -> Result<Option<u64>, String> {
    let value = value.trim();
    if value.is_empty() || value.eq_ignore_ascii_case("default") {
        return Ok(None);
    }
    let value = value.strip_suffix(suffix).map(str::trim).unwrap_or(value);
    let parsed = value
        .parse::<u64>()
        .map_err(|_| format!("{label} must be a valid number"))?;
    if parsed == 0 {
        return Err(format!("{label} must be greater than zero"));
    }
    Ok(Some(parsed))
}

fn parse_required_duration(value: &str, label: &str, suffix: &str) -> Result<u64, String> {
    let value = value.trim();
    let value = value.strip_suffix(suffix).map(str::trim).unwrap_or(value);
    let parsed = value
        .parse::<u64>()
        .map_err(|_| format!("{label} must be a valid number"))?;
    if parsed == 0 {
        return Err(format!("{label} must be greater than zero"));
    }
    Ok(parsed)
}

fn candidate_updated_config(
    config: &Config,
    original_name: &str,
    form: RouteForm,
) -> Result<Config, String> {
    let original_name = original_name.trim();
    let index = config
        .routes
        .iter()
        .position(|route| route.name == original_name)
        .ok_or_else(|| format!("route '{original_name}' was not found"))?;

    let mut candidate = config.clone();
    candidate.routes.remove(index);
    let updated_route = route_forms::candidate_route(form)?;
    candidate.routes.insert(index, updated_route);
    candidate.validate().map_err(|error| error.to_string())?;
    Ok(candidate)
}

fn render_write_denied(operation: &str, name: String, message: &str) -> Response {
    if operation == "stage-upstream" {
        render_upstream_stage(
            StatusCode::FORBIDDEN,
            false,
            name,
            String::new(),
            message.to_owned(),
        )
    } else {
        render_route_stage(
            StatusCode::FORBIDDEN,
            false,
            name,
            String::new(),
            message.to_owned(),
        )
    }
}

fn render_upstream_stage(
    status: StatusCode,
    success: bool,
    upstream_name: String,
    revision: String,
    message: String,
) -> Response {
    render_route_stage(
        status,
        success,
        format!("upstream {upstream_name}"),
        revision,
        message,
    )
}

fn render_route_stage(
    status: StatusCode,
    success: bool,
    route_name: String,
    revision: String,
    message: String,
) -> Response {
    match (RouteStageTemplate {
        success,
        route_name,
        revision,
        message,
    })
    .render()
    {
        Ok(html) => (status, Html(html)).into_response(),
        Err(error) => {
            log::error!("failed to render route stage result: {error}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use lungyam_core::config::{
        AdminConfig, Config, RouteConfig, RoutePolicies, ServerConfig, UpstreamConfig,
    };

    use super::{
        StageRouteForm, candidate_updated_config, candidate_upstream_config,
        parse_optional_duration,
    };
    use crate::route_forms::RouteForm;

    #[test]
    fn update_preserves_original_route_order() {
        let mut config = test_config();
        let mut second = config.routes[0].clone();
        second.name = "second".to_owned();
        second.path = "/second".to_owned();
        config.routes.push(second);

        let updated =
            candidate_updated_config(&config, "api-route", route_form("renamed", "/updated"))
                .expect("valid update");

        assert_eq!(updated.routes[0].name, "renamed");
        assert_eq!(updated.routes[1].name, "second");
    }

    #[test]
    fn upstream_candidate_parses_form_values_and_validates() {
        let config = test_config();
        let form = StageRouteForm {
            operation: "validate-upstream".to_owned(),
            original_name: String::new(),
            route: route_form("unused", "/"),
            upstream_original_name: String::new(),
            upstream_name: "canary".to_owned(),
            endpoints: "127.0.0.1:4000\n127.0.0.1:4001".to_owned(),
            connect_timeout: "2500 ms".to_owned(),
            read_timeout: "Default".to_owned(),
            write_timeout: "5000".to_owned(),
            health_check_interval: "7 s".to_owned(),
        };
        let candidate = candidate_upstream_config(&config, &form).expect("valid upstream");
        let upstream = &candidate.upstreams["canary"];
        assert_eq!(upstream.endpoints.len(), 2);
        assert_eq!(upstream.connect_timeout_ms, Some(2500));
        assert_eq!(upstream.read_timeout_ms, None);
        assert_eq!(upstream.write_timeout_ms, Some(5000));
        assert_eq!(upstream.health_check_interval_seconds, 7);
    }

    #[test]
    fn optional_duration_accepts_default_and_rejects_zero() {
        assert_eq!(
            parse_optional_duration("Default", "timeout", "ms"),
            Ok(None)
        );
        assert_eq!(
            parse_optional_duration("1500 ms", "timeout", "ms"),
            Ok(Some(1500))
        );
        assert!(parse_optional_duration("0", "timeout", "ms").is_err());
    }

    fn route_form(name: &str, path: &str) -> RouteForm {
        RouteForm {
            csrf_token: String::new(),
            name: name.to_owned(),
            host: String::new(),
            path: path.to_owned(),
            methods: "GET".to_owned(),
            upstream: "api".to_owned(),
            priority: "5".to_owned(),
            request_add_headers: String::new(),
            request_remove_headers: String::new(),
            response_add_headers: String::new(),
            response_remove_headers: String::new(),
            rate_limit_requests: String::new(),
            rate_limit_window_seconds: String::new(),
            max_request_body_bytes: String::new(),
        }
    }

    fn test_config() -> Config {
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
        Config {
            server: ServerConfig {
                listen: "127.0.0.1:8080".to_owned(),
            },
            admin: AdminConfig {
                enabled: true,
                listen: "127.0.0.1:9090".to_owned(),
                read_only: false,
            },
            upstreams,
            routes: vec![RouteConfig {
                name: "api-route".to_owned(),
                host: None,
                path: "/api".to_owned(),
                methods: vec!["GET".to_owned()],
                upstream: "api".to_owned(),
                priority: 0,
                policies: RoutePolicies::default(),
            }],
        }
    }
}
