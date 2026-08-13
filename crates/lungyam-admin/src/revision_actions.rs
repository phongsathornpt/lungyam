use axum::{
    Form,
    extract::State,
    http::StatusCode,
    response::{Html, IntoResponse, Response},
};
use lungyam_core::lifecycle::FileConfigLifecycle;
use serde::Deserialize;

use crate::{AdminState, security};

#[derive(Debug, Deserialize)]
pub(super) struct ActivatePendingForm {
    #[serde(default)]
    csrf_token: String,
}

pub(super) async fn activate_pending(
    State(state): State<AdminState>,
    Form(form): Form<ActivatePendingForm>,
) -> Response {
    let config = state.runtime.config();
    if config.admin.read_only {
        return error(StatusCode::FORBIDDEN, "Admin is configured as read-only.");
    }
    if !security::writes_enabled(&config) {
        return error(StatusCode::FORBIDDEN, "Admin writes require a loopback listener.");
    }
    if !security::csrf_token().verify(&form.csrf_token) {
        return error(StatusCode::FORBIDDEN, "CSRF token validation failed.");
    }
    let Some(config_path) = state.config_path.as_ref() else {
        return error(StatusCode::SERVICE_UNAVAILABLE, "Config path is unavailable.");
    };

    let lifecycle = FileConfigLifecycle::new(config_path);
    let activation = match lifecycle.activate_pending() {
        Ok(result) => result,
        Err(err) => {
            log::warn!("pending revision activation rejected: {err}");
            return error(StatusCode::UNPROCESSABLE_ENTITY, "Pending revision activation failed.");
        }
    };

    if activation.restart_required {
        return restart_required(activation.revision);
    }

    let revision = match lifecycle.revision_store().load(activation.revision) {
        Ok(revision) => revision,
        Err(err) => {
            log::error!("activated revision reload failed: {err}");
            return reconciliation_error(activation.revision);
        }
    };
    match state.runtime.apply_route_config(&revision.config) {
        Ok(()) => live_applied(activation.revision),
        Err(err) => {
            log::error!("activated revision runtime apply failed: {err}");
            reconciliation_error(activation.revision)
        }
    }
}

fn live_applied(revision: u64) -> Response {
    let html = format!("<div class=\"validation-card validation-success\"><strong>Revision #{revision:06} activated and applied live</strong><p>New requests now use the updated route matching and policies. In-flight requests keep their pinned snapshot.</p><p><a href=\"/admin/revisions\">Refresh revision status</a></p></div>");
    (StatusCode::OK, Html(html)).into_response()
}

fn restart_required(revision: u64) -> Response {
    let html = format!("<div class=\"validation-card validation-neutral\"><strong>Revision #{revision:06} activated; restart required</strong><p>The active configuration file was updated, but the running proxy remains unchanged because this revision contains structural changes.</p><p><a href=\"/admin/revisions\">Refresh revision status</a></p></div>");
    (StatusCode::OK, Html(html)).into_response()
}

fn reconciliation_error(revision: u64) -> Response {
    let html = format!("<div class=\"validation-card validation-error\"><strong>Revision #{revision:06} activated, but live apply failed</strong><p>The configuration on disk is active while the proxy still uses its previous runtime snapshot. Restart Lungyam to reconcile runtime state.</p></div>");
    (StatusCode::INTERNAL_SERVER_ERROR, Html(html)).into_response()
}

fn error(status: StatusCode, message: &str) -> Response {
    (status, Html(format!("<div class=\"validation-card validation-error\"><strong>Revision was not activated</strong><p>{message}</p></div>"))).into_response()
}
