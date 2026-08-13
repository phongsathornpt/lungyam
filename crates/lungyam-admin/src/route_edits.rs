use askama::Template;
use axum::{
    Form,
    extract::{Query, State},
    http::StatusCode,
    response::Response,
};
use lungyam_core::config::{Config, HeaderTransform, RouteConfig};
use serde::Deserialize;

use crate::{AdminState, mutation, route_forms, security};

#[derive(Debug, Deserialize)]
pub(super) struct EditRouteQuery {
    name: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct UpdateRouteForm {
    original_name: String,
    #[serde(flatten)]
    route: route_forms::RouteForm,
}

#[derive(Debug, Deserialize)]
pub(super) struct DeleteRouteForm {
    csrf_token: String,
    name: String,
}

#[derive(Clone, Debug)]
struct UpstreamOption {
    name: String,
    selected: bool,
}

#[derive(Template)]
#[template(path = "route-edit.html")]
struct RouteEditTemplate {
    overview_active: bool,
    routes_active: bool,
    original_name: String,
    name: String,
    host: String,
    path: String,
    methods: String,
    priority: i32,
    upstreams: Vec<UpstreamOption>,
    request_add_headers: String,
    request_remove_headers: String,
    response_add_headers: String,
    response_remove_headers: String,
    rate_limit_requests: String,
    rate_limit_window_seconds: String,
    max_request_body_bytes: String,
    csrf_token: String,
    writes_enabled: bool,
}

pub(super) async fn edit_route_page(
    State(state): State<AdminState>,
    Query(query): Query<EditRouteQuery>,
) -> Response {
    let config = state.runtime.config();
    match render_edit(&config, &query.name) {
        Ok(html) => axum::response::Html(html).into_response(),
        Err(message) => (StatusCode::NOT_FOUND, message).into_response(),
    }
}

pub(super) async fn validate_route_update(
    State(state): State<AdminState>,
    Form(form): Form<UpdateRouteForm>,
) -> Response {
    let route_name = form.route.name.trim().to_owned();
    match candidate_updated_config(&state.runtime.config(), &form.original_name, form.route) {
        Ok(_) => route_forms::render_validation_message(
            true,
            route_name,
            "Candidate update passed the active Lungyam configuration validation rules.".to_owned(),
        ),
        Err(message) => route_forms::render_validation_message(false, route_name, message),
    }
    .map(axum::response::Html)
    .map(axum::response::IntoResponse::into_response)
    .unwrap_or_else(|error| {
        log::error!("failed to render route update validation: {error}");
        StatusCode::INTERNAL_SERVER_ERROR.into_response()
    })
}

pub(super) async fn stage_route_update(
    State(state): State<AdminState>,
    Form(form): Form<UpdateRouteForm>,
) -> Response {
    let route_name = form.route.name.trim().to_owned();
    let config_path = match mutation::authorize(&state, &form.route.csrf_token, &route_name) {
        Ok(path) => path,
        Err(response) => return response,
    };
    let candidate = match candidate_updated_config(&state.runtime.config(), &form.original_name, form.route) {
        Ok(candidate) => candidate,
        Err(message) => return mutation::error(StatusCode::UNPROCESSABLE_ENTITY, &route_name, &message),
    };
    mutation::stage(
        config_path,
        &candidate,
        route_name.clone(),
        format!("stage route update '{}' -> '{route_name}'", form.original_name),
    )
}

pub(super) async fn stage_route_delete(
    State(state): State<AdminState>,
    Form(form): Form<DeleteRouteForm>,
) -> Response {
    let route_name = form.name.trim().to_owned();
    let config_path = match mutation::authorize(&state, &form.csrf_token, &route_name) {
        Ok(path) => path,
        Err(response) => return response,
    };
    let candidate = match candidate_deleted_config(&state.runtime.config(), &route_name) {
        Ok(candidate) => candidate,
        Err(message) => return mutation::error(StatusCode::UNPROCESSABLE_ENTITY, &route_name, &message),
    };
    mutation::stage(
        config_path,
        &candidate,
        route_name.clone(),
        format!("stage route delete '{route_name}'"),
    )
}

fn render_edit(config: &Config, name: &str) -> Result<String, String> {
    let route = config
        .routes
        .iter()
        .find(|route| route.name == name)
        .ok_or_else(|| format!("route '{name}' was not found"))?;
    let rate_limit_requests = route
        .policies
        .rate_limit
        .as_ref()
        .map(|limit| limit.requests.to_string())
        .unwrap_or_default();
    let rate_limit_window_seconds = route
        .policies
        .rate_limit
        .as_ref()
        .map(|limit| limit.window_seconds.to_string())
        .unwrap_or_default();

    RouteEditTemplate {
        overview_active: false,
        routes_active: true,
        original_name: route.name.clone(),
        name: route.name.clone(),
        host: route.host.clone().unwrap_or_default(),
        path: route.path.clone(),
        methods: route.methods.join(", "),
        priority: route.priority,
        upstreams: config
            .upstreams
            .keys()
            .map(|name| UpstreamOption {
                name: name.clone(),
                selected: *name == route.upstream,
            })
            .collect(),
        request_add_headers: render_added_headers(&route.policies.request_headers),
        request_remove_headers: route.policies.request_headers.remove.join(", "),
        response_add_headers: render_added_headers(&route.policies.response_headers),
        response_remove_headers: route.policies.response_headers.remove.join(", "),
        rate_limit_requests,
        rate_limit_window_seconds,
        max_request_body_bytes: route
            .policies
            .max_request_body_bytes
            .map(|value| value.to_string())
            .unwrap_or_default(),
        csrf_token: security::csrf_token().expose().to_owned(),
        writes_enabled: security::writes_enabled(config),
    }
    .render()
    .map_err(|error| error.to_string())
}

fn candidate_updated_config(
    config: &Config,
    original_name: &str,
    form: route_forms::RouteForm,
) -> Result<Config, String> {
    let index = config
        .routes
        .iter()
        .position(|route| route.name == original_name)
        .ok_or_else(|| format!("route '{original_name}' was not found"))?;

    let mut without_original = config.clone();
    without_original.routes.remove(index);
    let mut with_candidate = route_forms::candidate_config(&without_original, form)?;
    let candidate_route = with_candidate
        .routes
        .pop()
        .ok_or_else(|| "candidate route was not produced".to_owned())?;
    without_original.routes.insert(index, candidate_route);
    without_original
        .validate()
        .map_err(|error| error.to_string())?;
    Ok(without_original)
}

fn candidate_deleted_config(config: &Config, name: &str) -> Result<Config, String> {
    let mut candidate = config.clone();
    let original_len = candidate.routes.len();
    candidate.routes.retain(|route| route.name != name);
    if candidate.routes.len() == original_len {
        return Err(format!("route '{name}' was not found"));
    }
    candidate.validate().map_err(|error| error.to_string())?;
    Ok(candidate)
}

fn render_added_headers(transform: &HeaderTransform) -> String {
    transform
        .add
        .iter()
        .map(|(name, value)| format!("{name}: {value}"))
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use lungyam_core::config::{
        AdminConfig, Config, RoutePolicies, ServerConfig, UpstreamConfig,
    };

    use super::{candidate_deleted_config, candidate_updated_config};
    use crate::route_forms::RouteForm;

    #[test]
    fn update_preserves_original_route_order_and_validates_rename() {
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
            routes: vec![lungyam_core::config::RouteConfig {
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
