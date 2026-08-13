use askama::Template;
use axum::{
    Form,
    extract::State,
    http::StatusCode,
    response::{Html, IntoResponse, Response},
};
use lungyam_core::lifecycle::FileConfigLifecycle;

use crate::{
    AdminState,
    route_forms::{self, RouteForm},
    security,
};

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
    Form(form): Form<RouteForm>,
) -> Response {
    let config = state.runtime.config();
    let route_name = form.name.trim().to_owned();

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
    if !security::csrf_token().verify(&form.csrf_token) {
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

    let candidate = match route_forms::candidate_config(&config, form) {
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

    match FileConfigLifecycle::new(config_path).stage(
        &candidate,
        Some("admin-web".to_owned()),
        Some(format!("stage route '{route_name}'")),
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
