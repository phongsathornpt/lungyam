use askama::Template;
use axum::{
    Form,
    extract::State,
    http::StatusCode,
    response::{Html, IntoResponse, Response},
};
use lungyam_core::{config::Config, lifecycle::FileConfigLifecycle};
use serde::Deserialize;

use crate::{
    AdminState,
    route_forms::{self, RouteForm},
    security,
};

#[derive(Debug, Deserialize)]
struct StageRouteForm {
    #[serde(default)]
    operation: String,
    #[serde(default)]
    original_name: String,
    #[serde(flatten)]
    route: RouteForm,
}

#[derive(Template)]
#[template(path = "fragments/route-stage.html")]
struct RouteStageTemplate {
    success: bool,
    route_name: String,
    revision: String,
    message: String,
}

pub(super) async fn stage_route(
    State(state): State<AdminState>,
    Form(form): Form<StageRouteForm>,
) -> Response {
    let config = state.runtime.config();
    let route_name = if form.operation == "delete" {
        form.original_name.trim().to_owned()
    } else {
        form.route.name.trim().to_owned()
    };

    if config.admin.read_only {
        return render_stage(
            StatusCode::FORBIDDEN,
            false,
            route_name,
            String::new(),
            "Admin is configured as read-only.".to_owned(),
        );
    }
    if !security::writes_enabled(&config) {
        return render_stage(
            StatusCode::FORBIDDEN,
            false,
            route_name,
            String::new(),
            "Admin writes require a loopback listener until authentication is configured."
                .to_owned(),
        );
    }
    if !security::csrf_token().verify(&form.route.csrf_token) {
        return render_stage(
            StatusCode::FORBIDDEN,
            false,
            route_name,
            String::new(),
            "CSRF token validation failed.".to_owned(),
        );
    }

    let Some(config_path) = state.config_path.as_ref() else {
        return render_stage(
            StatusCode::SERVICE_UNAVAILABLE,
            false,
            route_name,
            String::new(),
            "Config path is unavailable; staging is disabled for this admin router.".to_owned(),
        );
    };

    let operation = form.operation.trim();
    let candidate = match operation {
        "" | "create" => route_forms::candidate_config(&config, form.route),
        "update" => candidate_updated_config(&config, &form.original_name, form.route),
        "delete" => candidate_deleted_config(&config, &form.original_name),
        other => Err(format!("unsupported route mutation operation '{other}'")),
    };
    let candidate = match candidate {
        Ok(candidate) => candidate,
        Err(message) => {
            return render_stage(
                StatusCode::UNPROCESSABLE_ENTITY,
                false,
                route_name,
                String::new(),
                message,
            );
        }
    };

    let reason = match operation {
        "update" => format!(
            "stage route update '{}' -> '{route_name}'",
            form.original_name.trim()
        ),
        "delete" => format!("stage route delete '{}'", form.original_name.trim()),
        _ => format!("stage route '{route_name}'"),
    };

    match FileConfigLifecycle::new(config_path).stage(
        &candidate,
        Some("admin-web".to_owned()),
        Some(reason),
    ) {
        Ok(metadata) => render_stage(
            StatusCode::OK,
            true,
            route_name,
            format!("#{:06}", metadata.revision),
            String::new(),
        ),
        Err(error) => render_stage(
            StatusCode::INTERNAL_SERVER_ERROR,
            false,
            route_name,
            String::new(),
            error.to_string(),
        ),
    }
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

fn candidate_deleted_config(config: &Config, original_name: &str) -> Result<Config, String> {
    let original_name = original_name.trim();
    let mut candidate = config.clone();
    let original_len = candidate.routes.len();
    candidate.routes.retain(|route| route.name != original_name);
    if candidate.routes.len() == original_len {
        return Err(format!("route '{original_name}' was not found"));
    }
    candidate.validate().map_err(|error| error.to_string())?;
    Ok(candidate)
}

fn render_stage(
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

    use super::{candidate_deleted_config, candidate_updated_config};
    use crate::route_forms::RouteForm;

    #[test]
    fn update_preserves_original_route_order() {
        let mut config = test_config();
        let mut second = config.routes[0].clone();
        second.name = "second".to_owned();
        second.path = "/second".to_owned();
        config.routes.push(second);

        let updated = candidate_updated_config(
            &config,
            "api",
            RouteForm {
                csrf_token: String::new(),
                name: "renamed".to_owned(),
                host: String::new(),
                path: "/updated".to_owned(),
                methods: "POST".to_owned(),
                upstream: "api".to_owned(),
                priority: "5".to_owned(),
                request_add_headers: String::new(),
                request_remove_headers: String::new(),
                response_add_headers: String::new(),
                response_remove_headers: String::new(),
                rate_limit_requests: String::new(),
                rate_limit_window_seconds: String::new(),
                max_request_body_bytes: String::new(),
            },
        )
        .expect("valid update");

        assert_eq!(updated.routes[0].name, "renamed");
        assert_eq!(updated.routes[1].name, "second");
    }

    #[test]
    fn delete_requires_existing_route() {
        let config = test_config();
        let deleted = candidate_deleted_config(&config, "api").expect("delete route");
        assert!(deleted.routes.is_empty());
        assert!(candidate_deleted_config(&config, "missing").is_err());
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
                name: "api".to_owned(),
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
