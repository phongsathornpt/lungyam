use askama::Template;
use axum::{Form, extract::{Query, State}, http::StatusCode, response::{Html, IntoResponse, Response}};
use lungyam_core::config::{Config, HeaderTransform};
use serde::Deserialize;
use crate::{AdminState, mutation, route_forms, security};

#[derive(Debug, Deserialize)] pub(super) struct EditRouteQuery { name: String }
#[derive(Debug, Deserialize)] pub(super) struct UpdateRouteForm { original_name: String, #[serde(flatten)] route: route_forms::RouteForm }
#[derive(Debug, Deserialize)] pub(super) struct DeleteRouteForm { csrf_token: String, name: String }
#[derive(Clone, Debug)] struct UpstreamOption { name: String, selected: bool }

#[derive(Template)]
#[template(path = "route-edit.html")]
struct RouteEditTemplate { overview_active: bool, routes_active: bool, original_name: String, name: String, host: String, path: String, methods: String, priority: i32, upstreams: Vec<UpstreamOption>, request_add_headers: String, request_remove_headers: String, response_add_headers: String, response_remove_headers: String, rate_limit_requests: String, rate_limit_window_seconds: String, max_request_body_bytes: String, csrf_token: String, writes_enabled: bool }

#[derive(Template)]
#[template(path = "fragments/route-validation.html")]
struct RouteUpdateValidationTemplate { valid: bool, route_name: String, message: String }

pub(super) async fn edit_route_page(State(state): State<AdminState>, Query(query): Query<EditRouteQuery>) -> Response {
    match render_edit(&state.runtime.config(), &query.name) { Ok(html) => Html(html).into_response(), Err(message) => (StatusCode::NOT_FOUND, message).into_response() }
}

pub(super) async fn validate_route_update(State(state): State<AdminState>, Form(form): Form<UpdateRouteForm>) -> Response {
    let route_name = form.route.name.trim().to_owned();
    let (valid, message) = match candidate_updated_config(&state.runtime.config(), &form.original_name, form.route) { Ok(_) => (true, "Candidate update passed the active Lungyam configuration validation rules.".to_owned()), Err(message) => (false, message) };
    match (RouteUpdateValidationTemplate { valid, route_name, message }).render() { Ok(html) => Html(html).into_response(), Err(error) => { log::error!("failed to render route update validation: {error}"); StatusCode::INTERNAL_SERVER_ERROR.into_response() } }
}

pub(super) async fn stage_route_update(State(state): State<AdminState>, Form(form): Form<UpdateRouteForm>) -> Response {
    let route_name = form.route.name.trim().to_owned();
    let config_path = match mutation::authorize(&state, &form.route.csrf_token, &route_name) { Ok(path) => path, Err(response) => return response };
    let candidate = match candidate_updated_config(&state.runtime.config(), &form.original_name, form.route) { Ok(candidate) => candidate, Err(message) => return mutation::error(StatusCode::UNPROCESSABLE_ENTITY, &route_name, &message) };
    mutation::stage(config_path, &candidate, route_name.clone(), format!("stage route update '{}' -> '{route_name}'", form.original_name))
}

pub(super) async fn stage_route_delete(State(state): State<AdminState>, Form(form): Form<DeleteRouteForm>) -> Response {
    let route_name = form.name.trim().to_owned();
    let config_path = match mutation::authorize(&state, &form.csrf_token, &route_name) { Ok(path) => path, Err(response) => return response };
    let candidate = match candidate_deleted_config(&state.runtime.config(), &route_name) { Ok(candidate) => candidate, Err(message) => return mutation::error(StatusCode::UNPROCESSABLE_ENTITY, &route_name, &message) };
    mutation::stage(config_path, &candidate, route_name.clone(), format!("stage route delete '{route_name}'"))
}

fn render_edit(config: &Config, name: &str) -> Result<String, String> {
    let route = config.routes.iter().find(|route| route.name == name).ok_or_else(|| format!("route '{name}' was not found"))?;
    RouteEditTemplate { overview_active: false, routes_active: true, original_name: route.name.clone(), name: route.name.clone(), host: route.host.clone().unwrap_or_default(), path: route.path.clone(), methods: route.methods.join(", "), priority: route.priority, upstreams: config.upstreams.keys().map(|name| UpstreamOption { name: name.clone(), selected: *name == route.upstream }).collect(), request_add_headers: render_added_headers(&route.policies.request_headers), request_remove_headers: route.policies.request_headers.remove.join(", "), response_add_headers: render_added_headers(&route.policies.response_headers), response_remove_headers: route.policies.response_headers.remove.join(", "), rate_limit_requests: route.policies.rate_limit.as_ref().map(|limit| limit.requests.to_string()).unwrap_or_default(), rate_limit_window_seconds: route.policies.rate_limit.as_ref().map(|limit| limit.window_seconds.to_string()).unwrap_or_default(), max_request_body_bytes: route.policies.max_request_body_bytes.map(|value| value.to_string()).unwrap_or_default(), csrf_token: security::csrf_token().expose().to_owned(), writes_enabled: security::writes_enabled(config) }.render().map_err(|error| error.to_string())
}

fn candidate_updated_config(config: &Config, original_name: &str, form: route_forms::RouteForm) -> Result<Config, String> {
    let index = config.routes.iter().position(|route| route.name == original_name).ok_or_else(|| format!("route '{original_name}' was not found"))?;
    let mut without_original = config.clone(); without_original.routes.remove(index);
    let mut with_candidate = route_forms::candidate_config(&without_original, form)?;
    let candidate_route = with_candidate.routes.pop().ok_or_else(|| "candidate route was not produced".to_owned())?;
    without_original.routes.insert(index, candidate_route); without_original.validate().map_err(|error| error.to_string())?; Ok(without_original)
}

fn candidate_deleted_config(config: &Config, name: &str) -> Result<Config, String> {
    let mut candidate = config.clone(); let original_len = candidate.routes.len(); candidate.routes.retain(|route| route.name != name); if candidate.routes.len() == original_len { return Err(format!("route '{name}' was not found")); } candidate.validate().map_err(|error| error.to_string())?; Ok(candidate)
}

fn render_added_headers(transform: &HeaderTransform) -> String { transform.add.iter().map(|(name, value)| format!("{name}: {value}")).collect::<Vec<_>>().join("\n") }
