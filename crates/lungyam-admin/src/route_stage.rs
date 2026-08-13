use axum::{Form, extract::State, http::StatusCode, response::Response};

use crate::{
    AdminState, mutation,
    route_forms::{self, RouteForm},
};

pub(super) async fn stage_route(
    State(state): State<AdminState>,
    Form(form): Form<RouteForm>,
) -> Response {
    let route_name = form.name.trim().to_owned();
    let config_path = match mutation::authorize(&state, &form.csrf_token, &route_name) {
        Ok(path) => path,
        Err(response) => return response,
    };
    let candidate = match route_forms::candidate_config(&state.runtime.config(), form) {
        Ok(candidate) => candidate,
        Err(message) => {
            return mutation::error(StatusCode::UNPROCESSABLE_ENTITY, &route_name, &message);
        }
    };
    mutation::stage(
        config_path,
        &candidate,
        route_name.clone(),
        format!("stage route '{route_name}'"),
    )
}
