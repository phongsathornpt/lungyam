//! Lightweight server-rendered control plane for Lungyam.

use std::{io, net::TcpListener as StdTcpListener, sync::Arc, thread::JoinHandle, time::Instant};

use askama::Template;
use axum::{
    Router,
    extract::State,
    http::{StatusCode, header},
    response::{Html, IntoResponse, Response},
    routing::get,
};
use lungyam_core::config::Config;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActiveConfigSummary {
    pub proxy_listen: String,
    pub admin_listen: String,
    pub route_count: usize,
    pub upstream_count: usize,
    pub endpoint_count: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuntimeSnapshot {
    pub uptime_seconds: u64,
    pub active_config: ActiveConfigSummary,
}

/// Read-only runtime state exposed to the admin control plane.
#[derive(Debug)]
pub struct RuntimeStatus {
    started_at: Instant,
    active_config: ActiveConfigSummary,
}

impl RuntimeStatus {
    #[must_use]
    pub fn from_config(config: &Config) -> Self {
        let endpoint_count = config
            .upstreams
            .values()
            .map(|upstream| upstream.endpoints.len())
            .sum();

        Self {
            started_at: Instant::now(),
            active_config: ActiveConfigSummary {
                proxy_listen: config.server.listen.clone(),
                admin_listen: config.admin.listen.clone(),
                route_count: config.routes.len(),
                upstream_count: config.upstreams.len(),
                endpoint_count,
            },
        }
    }

    #[must_use]
    pub fn snapshot(&self) -> RuntimeSnapshot {
        RuntimeSnapshot {
            uptime_seconds: self.started_at.elapsed().as_secs(),
            active_config: self.active_config.clone(),
        }
    }
}

#[derive(Clone)]
struct AdminState {
    runtime: Arc<RuntimeStatus>,
}

#[derive(Template)]
#[template(path = "dashboard.html")]
struct DashboardTemplate {
    proxy_listen: String,
    admin_listen: String,
    route_count: usize,
    upstream_count: usize,
    endpoint_count: usize,
    uptime: String,
}

/// Handle that keeps ownership of the admin server thread.
#[derive(Debug)]
pub struct AdminHandle {
    _thread: JoinHandle<()>,
}

/// Builds the read-only admin router with a fresh runtime status snapshot source.
pub fn router(config: Config) -> Router {
    let runtime = Arc::new(RuntimeStatus::from_config(&config));
    router_with_status(runtime)
}

/// Builds the read-only admin router around shared runtime status.
pub fn router_with_status(runtime: Arc<RuntimeStatus>) -> Router {
    Router::new()
        .route("/admin", get(dashboard))
        .route("/admin/health", get(health))
        .route("/admin/assets/lungyam.css", get(stylesheet))
        .with_state(AdminState { runtime })
}

/// Starts the independent admin listener in its own runtime thread.
pub fn start(config: Config) -> io::Result<AdminHandle> {
    let runtime = Arc::new(RuntimeStatus::from_config(&config));
    start_with_status(config, runtime)
}

/// Starts the admin listener with a caller-owned shared runtime status source.
pub fn start_with_status(config: Config, runtime: Arc<RuntimeStatus>) -> io::Result<AdminHandle> {
    let listen = config.admin.listen.clone();
    let listener = StdTcpListener::bind(&listen)?;
    listener.set_nonblocking(true)?;

    let thread = std::thread::Builder::new()
        .name("lungyam-admin".to_owned())
        .spawn(move || {
            let runtime_thread = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .expect("failed to build admin Tokio runtime");

            runtime_thread.block_on(async move {
                let listener = tokio::net::TcpListener::from_std(listener)
                    .expect("failed to register admin listener with Tokio");
                log::info!("Lungyam admin listening on {listen}");
                axum::serve(listener, router_with_status(runtime))
                    .await
                    .expect("admin server exited with an error");
            });
        })?;

    Ok(AdminHandle { _thread: thread })
}

async fn dashboard(State(state): State<AdminState>) -> Response {
    let snapshot = state.runtime.snapshot();
    let active = snapshot.active_config;
    let template = DashboardTemplate {
        proxy_listen: active.proxy_listen,
        admin_listen: active.admin_listen,
        route_count: active.route_count,
        upstream_count: active.upstream_count,
        endpoint_count: active.endpoint_count,
        uptime: format_uptime(snapshot.uptime_seconds),
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

fn format_uptime(seconds: u64) -> String {
    let hours = seconds / 3_600;
    let minutes = (seconds % 3_600) / 60;
    let seconds = seconds % 60;

    if hours > 0 {
        format!("{hours}h {minutes}m {seconds}s")
    } else if minutes > 0 {
        format!("{minutes}m {seconds}s")
    } else {
        format!("{seconds}s")
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::BTreeMap, sync::Arc};

    use askama::Template;
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use lungyam_core::config::{
        AdminConfig, Config, RouteConfig, RoutePolicies, ServerConfig, UpstreamConfig,
    };
    use tower::ServiceExt;

    use super::{DashboardTemplate, RuntimeStatus, format_uptime, router_with_status};

    #[test]
    fn runtime_status_captures_active_config_summary() {
        let config = test_config();
        let snapshot = RuntimeStatus::from_config(&config).snapshot();

        assert_eq!(snapshot.active_config.proxy_listen, "0.0.0.0:8080");
        assert_eq!(snapshot.active_config.admin_listen, "127.0.0.1:9090");
        assert_eq!(snapshot.active_config.route_count, 1);
        assert_eq!(snapshot.active_config.upstream_count, 1);
        assert_eq!(snapshot.active_config.endpoint_count, 2);
    }

    #[test]
    fn dashboard_template_renders_runtime_summary() {
        let snapshot = RuntimeStatus::from_config(&test_config()).snapshot();
        let active = snapshot.active_config;
        let template = DashboardTemplate {
            proxy_listen: active.proxy_listen,
            admin_listen: active.admin_listen,
            route_count: active.route_count,
            upstream_count: active.upstream_count,
            endpoint_count: active.endpoint_count,
            uptime: format_uptime(snapshot.uptime_seconds),
        };
        let html = template.render().expect("dashboard should render");

        assert!(html.contains("Lungyam Admin"));
        assert!(html.contains("0.0.0.0:8080"));
        assert!(html.contains("127.0.0.1:9090"));
        assert!(html.contains(">1<"));
        assert!(html.contains(">2<"));
        assert!(html.contains("Uptime"));
    }

    #[test]
    fn admin_routes_respond_over_http_service() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime");

        runtime.block_on(async {
            let status = Arc::new(RuntimeStatus::from_config(&test_config()));
            let app = router_with_status(status);

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

    #[test]
    fn formats_uptime_compactly() {
        assert_eq!(format_uptime(8), "8s");
        assert_eq!(format_uptime(68), "1m 8s");
        assert_eq!(format_uptime(3_668), "1h 1m 8s");
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
