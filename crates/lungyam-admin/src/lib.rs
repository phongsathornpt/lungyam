//! Lightweight server-rendered control plane for Lungyam.

use std::{io, net::TcpListener as StdTcpListener, thread::JoinHandle};

use askama::Template;
use axum::{
    Router,
    extract::State,
    http::{StatusCode, header},
    response::{Html, IntoResponse, Response},
    routing::get,
};
use lungyam_core::config::Config;

#[derive(Clone)]
struct AdminState {
    config: Config,
}

#[derive(Template)]
#[template(path = "dashboard.html")]
struct DashboardTemplate {
    proxy_listen: String,
    admin_listen: String,
    route_count: usize,
    upstream_count: usize,
    endpoint_count: usize,
}

/// Handle that keeps ownership of the admin server thread.
#[derive(Debug)]
pub struct AdminHandle {
    _thread: JoinHandle<()>,
}

/// Builds the read-only admin router.
pub fn router(config: Config) -> Router {
    Router::new()
        .route("/admin", get(dashboard))
        .route("/admin/health", get(health))
        .route("/admin/assets/lungyam.css", get(stylesheet))
        .with_state(AdminState { config })
}

/// Starts the independent admin listener in its own runtime thread.
pub fn start(config: Config) -> io::Result<AdminHandle> {
    let listen = config.admin.listen.clone();
    let listener = StdTcpListener::bind(&listen)?;
    listener.set_nonblocking(true)?;

    let thread = std::thread::Builder::new()
        .name("lungyam-admin".to_owned())
        .spawn(move || {
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .expect("failed to build admin Tokio runtime");

            runtime.block_on(async move {
                let listener = tokio::net::TcpListener::from_std(listener)
                    .expect("failed to register admin listener with Tokio");
                log::info!("Lungyam admin listening on {listen}");
                axum::serve(listener, router(config))
                    .await
                    .expect("admin server exited with an error");
            });
        })?;

    Ok(AdminHandle { _thread: thread })
}

async fn dashboard(State(state): State<AdminState>) -> Response {
    let endpoint_count = state
        .config
        .upstreams
        .values()
        .map(|upstream| upstream.endpoints.len())
        .sum();
    let template = DashboardTemplate {
        proxy_listen: state.config.server.listen.clone(),
        admin_listen: state.config.admin.listen.clone(),
        route_count: state.config.routes.len(),
        upstream_count: state.config.upstreams.len(),
        endpoint_count,
    };

    match template.render() {
        Ok(html) => Html(html).into_response(),
        Err(error) => {
            log::error!("failed to render admin dashboard: {error}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "failed to render dashboard\n",
            )
                .into_response()
        }
    }
}

async fn health() -> &'static str {
    "ok\n"
}

async fn stylesheet() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/css; charset=utf-8")],
        include_str!("../static/lungyam.css"),
    )
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use askama::Template;
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use lungyam_core::config::{
        AdminConfig, Config, RouteConfig, RoutePolicies, ServerConfig, UpstreamConfig,
    };
    use tower::ServiceExt;

    use super::{DashboardTemplate, router};

    #[test]
    fn dashboard_template_renders_runtime_summary() {
        let config = test_config();
        let template = DashboardTemplate {
            proxy_listen: config.server.listen,
            admin_listen: config.admin.listen,
            route_count: config.routes.len(),
            upstream_count: config.upstreams.len(),
            endpoint_count: config
                .upstreams
                .values()
                .map(|upstream| upstream.endpoints.len())
                .sum(),
        };
        let html = template.render().expect("dashboard should render");

        assert!(html.contains("Lungyam Admin"));
        assert!(html.contains("0.0.0.0:8080"));
        assert!(html.contains("127.0.0.1:9090"));
        assert!(html.contains(">1<"));
        assert!(html.contains(">2<"));
    }

    #[test]
    fn admin_routes_respond_over_http_service() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime");

        runtime.block_on(async {
            let app = router(test_config());

            let health = app
                .clone()
                .oneshot(
                    Request::builder()
                        .uri("/admin/health")
                        .body(Body::empty())
                        .expect("health request"),
                )
                .await
                .expect("health response");
            assert_eq!(health.status(), StatusCode::OK);

            let dashboard = app
                .oneshot(
                    Request::builder()
                        .uri("/admin")
                        .body(Body::empty())
                        .expect("dashboard request"),
                )
                .await
                .expect("dashboard response");
            assert_eq!(dashboard.status(), StatusCode::OK);
        });
    }

    fn test_config() -> Config {
        let mut upstreams = BTreeMap::new();
        upstreams.insert(
            "api".to_owned(),
            UpstreamConfig {
                endpoints: vec!["127.0.0.1:3000".to_owned(), "127.0.0.1:3001".to_owned()],
                connect_timeout_ms: None,
                read_timeout_ms: None,
                write_timeout_ms: None,
                health_check_interval_seconds: 5,
            },
        );

        Config {
            server: ServerConfig {
                listen: "0.0.0.0:8080".to_owned(),
            },
            admin: AdminConfig {
                enabled: true,
                listen: "127.0.0.1:9090".to_owned(),
            },
            upstreams,
            routes: vec![RouteConfig {
                name: "api".to_owned(),
                host: None,
                path: "/".to_owned(),
                methods: Vec::new(),
                upstream: "api".to_owned(),
                priority: 0,
                policies: RoutePolicies::default(),
            }],
        }
    }
}
