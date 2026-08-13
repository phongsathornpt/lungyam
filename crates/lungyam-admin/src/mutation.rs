use std::path::PathBuf;

use askama::Template;
use axum::{
    http::StatusCode,
    response::{Html, IntoResponse, Response},
};
use lungyam_core::{config::Config, lifecycle::FileConfigLifecycle};

use crate::{AdminState, security};

#[derive(Template)]
#[template(path = "fragments/route-stage.html")]
struct RouteStageTemplate {
    success: bool,
    route_name: String,
    revision: String,
    message: String,
}

pub(super) fn authorize(
    state: &AdminState,
    csrf_token: &str,
    route_name: &str,
) -> Result<PathBuf, Response> {
    let config = state.runtime.config();
    if config.admin.read_only {
        return Err(error(StatusCode::FORBIDDEN, route_name, "Admin is configured as read-only."));
    }
    if !security::writes_enabled(&config) {
        return Err(error(StatusCode::FORBIDDEN, route_name, "Admin writes require a loopback listener until authentication is configured."));
    }
    if !security::csrf_token().verify(csrf_token) {
        return Err(error(StatusCode::FORBIDDEN, route_name, "CSRF token validation failed."));
    }
    state.config_path.clone().ok_or_else(|| {
        error(StatusCode::SERVICE_UNAVAILABLE, route_name, "Config path is unavailable; staging is disabled for this admin router.")
    })
}

pub(super) fn stage(
    config_path: PathBuf,
    candidate: &Config,
    route_name: String,
    reason: String,
) -> Response {
    match FileConfigLifecycle::new(config_path).stage(candidate, Some("admin-web".to_owned()), Some(reason)) {
        Ok(metadata) => render(StatusCode::OK, true, route_name, format!("#{:06}", metadata.revision), String::new()),
        Err(error_value) => error(StatusCode::INTERNAL_SERVER_ERROR, &route_name, &error_value.to_string()),
    }
}

pub(super) fn error(status: StatusCode, route_name: &str, message: &str) -> Response {
    render(status, false, route_name.to_owned(), String::new(), message.to_owned())
}

fn render(
    status: StatusCode,
    success: bool,
    route_name: String,
    revision: String,
    message: String,
) -> Response {
    match (RouteStageTemplate { success, route_name, revision, message }).render() {
        Ok(html) => (status, Html(html)).into_response(),
        Err(error) => {
            log::error!("failed to render route stage result: {error}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}
