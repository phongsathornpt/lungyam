use std::collections::BTreeMap;

use askama::Template;
use axum::{
    extract::State,
    http::StatusCode,
    response::{Html, IntoResponse, Response},
};

use crate::AdminState;

#[derive(Clone, Debug)]
struct EndpointView {
    address: String,
    status: &'static str,
    status_class: &'static str,
}

#[derive(Clone, Debug)]
struct UpstreamView {
    name: String,
    endpoints: Vec<EndpointView>,
    connect_timeout: String,
    read_timeout: String,
    write_timeout: String,
    health_check_interval: String,
}

#[derive(Template)]
#[template(path = "upstreams.html")]
struct UpstreamsTemplate {
    overview_active: bool,
    routes_active: bool,
    upstream_count: usize,
    endpoint_count: usize,
    upstreams: Vec<UpstreamView>,
}

pub(super) async fn upstreams_page(State(state): State<AdminState>) -> Response {
    let config = state.runtime.config();
    let snapshot = state.runtime.snapshot();
    let health = snapshot
        .endpoint_health
        .into_iter()
        .map(|endpoint| ((endpoint.upstream, endpoint.endpoint), endpoint.healthy))
        .collect::<BTreeMap<_, _>>();

    let upstreams = config
        .upstreams
        .iter()
        .map(|(name, upstream)| UpstreamView {
            name: name.clone(),
            endpoints: upstream
                .endpoints
                .iter()
                .map(|address| {
                    let healthy = health
                        .get(&(name.clone(), address.clone()))
                        .copied()
                        .unwrap_or(false);
                    EndpointView {
                        address: address.clone(),
                        status: if healthy { "Healthy" } else { "Unhealthy" },
                        status_class: if healthy { "health-healthy" } else { "health-unhealthy" },
                    }
                })
                .collect(),
            connect_timeout: format_timeout(upstream.connect_timeout_ms),
            read_timeout: format_timeout(upstream.read_timeout_ms),
            write_timeout: format_timeout(upstream.write_timeout_ms),
            health_check_interval: format!("{} s", upstream.health_check_interval_seconds),
        })
        .collect::<Vec<_>>();
    let endpoint_count = upstreams.iter().map(|upstream| upstream.endpoints.len()).sum();

    match (UpstreamsTemplate {
        overview_active: false,
        routes_active: false,
        upstream_count: upstreams.len(),
        endpoint_count,
        upstreams,
    })
    .render()
    {
        Ok(html) => Html(html).into_response(),
        Err(error) => {
            log::error!("failed to render upstreams page: {error}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

fn format_timeout(timeout_ms: Option<u64>) -> String {
    timeout_ms.map_or_else(|| "Default".to_owned(), |value| format!("{value} ms"))
}

#[cfg(test)]
mod tests {
    use super::format_timeout;

    #[test]
    fn formats_optional_timeouts() {
        assert_eq!(format_timeout(None), "Default");
        assert_eq!(format_timeout(Some(2_500)), "2500 ms");
    }
}
