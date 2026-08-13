//! Lightweight server-rendered control plane for Lungyam.

mod revision_views;
mod route_forms;
mod route_simulator;
mod route_stage;
mod security;

use std::{io, net::TcpListener as StdTcpListener, path::PathBuf, sync::Arc, thread::JoinHandle};

use askama::Template;
use axum::{
    Form, Router,
    extract::{Query, State},
    http::{StatusCode, header},
    response::{Html, IntoResponse, Response},
    routing::{get, post},
};
use lungyam_core::{
    config::{Config, RouteConfig},
    routing::sort_routes,
    runtime::{EndpointHealth, RuntimeStatus},
};
use revision_views::DiffQuery;
use route_forms::RouteForm;
use route_simulator::RouteMatchForm;

#[derive(Clone)]
struct AdminState {
    runtime: Arc<RuntimeStatus>,
    config_path: Option<PathBuf>,
}

#[derive(Clone, Debug)]
struct EndpointHealthView {
    upstream: String,
    endpoint: String,
    status: &'static str,
    status_class: &'static str,
}

#[derive(Clone, Debug)]
struct RouteView {
    name: String,
    host: String,
    path: String,
    methods: String,
    upstream: String,
    priority: i32,
    header_operations: usize,
    rate_limit: String,
    body_limit: String,
}

#[derive(Template)]
#[template(path = "dashboard.html")]
struct DashboardTemplate {
    overview_active: bool,
    routes_active: bool,
    proxy_listen: String,
    admin_listen: String,
    admin_mode: String,
    route_count: usize,
    upstream_count: usize,
    endpoint_count: usize,
    uptime: String,
    endpoint_health: Vec<EndpointHealthView>,
}

#[derive(Template)]
#[template(path = "routes.html")]
struct RoutesTemplate {
    overview_active: bool,
    routes_active: bool,
    route_count: usize,
    routes: Vec<RouteView>,
}

#[derive(Template)]
#[template(path = "fragments/upstream-health.html")]
struct UpstreamHealthTemplate {
    endpoint_health: Vec<EndpointHealthView>,
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
    build_router(runtime, None)
}

/// Builds the admin router with access to filesystem-backed revision data.
pub fn router_with_status_and_config_path(
    runtime: Arc<RuntimeStatus>,
    config_path: PathBuf,
) -> Router {
    build_router(runtime, Some(config_path))
}

fn build_router(runtime: Arc<RuntimeStatus>, config_path: Option<PathBuf>) -> Router {
    Router::new()
        .route("/admin", get(dashboard))
        .route("/admin/routes", get(routes_page))
        .route("/admin/routes/new", get(new_route_page))
        .route("/admin/routes/validate", post(validate_route))
        .route("/admin/routes/stage", post(route_stage::stage_route))
        .route("/admin/routes/simulate", post(simulate_route))
        .route("/admin/revisions", get(revisions_page))
        .route("/admin/fragments/config-diff", get(config_diff_fragment))
        .route("/admin/health", get(health))
        .route(
            "/admin/fragments/upstream-health",
            get(upstream_health_fragment),
        )
        .route("/admin/assets/lungyam.css", get(stylesheet))
        .route("/admin/assets/htmx.min.js", get(htmx_asset))
        .with_state(AdminState {
            runtime,
            config_path,
        })
}

/// Starts the independent admin listener in its own runtime thread.
pub fn start(config: Config) -> io::Result<AdminHandle> {
    let runtime = Arc::new(RuntimeStatus::from_config(&config));
    start_with_status(config, runtime)
}

/// Starts the admin listener with a caller-owned shared runtime status source.
pub fn start_with_status(config: Config, runtime: Arc<RuntimeStatus>) -> io::Result<AdminHandle> {
    start_with_status_internal(config, runtime, None)
}

/// Starts the admin listener with access to filesystem-backed revision data.
pub fn start_with_status_and_config_path(
    config: Config,
    runtime: Arc<RuntimeStatus>,
    config_path: PathBuf,
) -> io::Result<AdminHandle> {
    start_with_status_internal(config, runtime, Some(config_path))
}

fn start_with_status_internal(
    config: Config,
    runtime: Arc<RuntimeStatus>,
    config_path: Option<PathBuf>,
) -> io::Result<AdminHandle> {
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
                axum::serve(listener, build_router(runtime, config_path))
                    .await
                    .expect("admin server exited with an error");
            });
        })?;

    Ok(AdminHandle { _thread: thread })
}

async fn dashboard(State(state): State<AdminState>) -> Response {
    let snapshot = state.runtime.snapshot();
    let active = snapshot.active_config;
    render_template(
        &DashboardTemplate {
            overview_active: true,
            routes_active: false,
            proxy_listen: active.proxy_listen,
            admin_listen: active.admin_listen,
            admin_mode: if active.admin_read_only {
                "Read-only".to_owned()
            } else {
                "Writes enabled".to_owned()
            },
            route_count: active.route_count,
            upstream_count: active.upstream_count,
            endpoint_count: active.endpoint_count,
            uptime: format_uptime(snapshot.uptime_seconds),
            endpoint_health: health_views(snapshot.endpoint_health),
        },
        "dashboard",
    )
}

async fn routes_page(State(state): State<AdminState>) -> Response {
    let routes = route_views(state.runtime.routes());
    render_template(
        &RoutesTemplate {
            overview_active: false,
            routes_active: true,
            route_count: routes.len(),
            routes,
        },
        "routes page",
    )
}

async fn new_route_page(State(state): State<AdminState>) -> Response {
    render_html_result(
        route_forms::render_new_route(&state.runtime.config()),
        "new route form",
    )
}

async fn validate_route(State(state): State<AdminState>, Form(form): Form<RouteForm>) -> Response {
    render_html_result(
        route_forms::render_validation(&state.runtime.config(), form),
        "route validation",
    )
}

async fn simulate_route(
    State(state): State<AdminState>,
    Form(form): Form<RouteMatchForm>,
) -> Response {
    render_html_result(
        route_simulator::render_simulation(&state.runtime.config(), form),
        "route simulation",
    )
}

async fn revisions_page(State(state): State<AdminState>) -> Response {
    render_revision_result(
        revision_views::render_revisions(state.config_path.as_deref()),
        "revisions page",
    )
}

async fn config_diff_fragment(
    State(state): State<AdminState>,
    Query(query): Query<DiffQuery>,
) -> Response {
    render_revision_result(
        revision_views::render_diff(state.config_path.as_deref(), query),
        "config diff",
    )
}

async fn upstream_health_fragment(State(state): State<AdminState>) -> Response {
    let snapshot = state.runtime.snapshot();
    render_template(
        &UpstreamHealthTemplate {
            endpoint_health: health_views(snapshot.endpoint_health),
        },
        "upstream health fragment",
    )
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

async fn htmx_asset() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/javascript; charset=utf-8")],
        include_str!("../vendor/htmx.min.js"),
    )
}

fn render_template(template: &impl Template, label: &str) -> Response {
    render_html_result(template.render(), label)
}

fn render_revision_result(result: Result<String, String>, label: &str) -> Response {
    match result {
        Ok(html) => Html(html).into_response(),
        Err(error) => {
            log::error!("failed to render admin {label}: {error}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "failed to render admin view\n",
            )
                .into_response()
        }
    }
}

fn render_html_result(result: askama::Result<String>, label: &str) -> Response {
    match result {
        Ok(html) => Html(html).into_response(),
        Err(error) => {
            log::error!("failed to render admin {label}: {error}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "failed to render admin view\n",
            )
                .into_response()
        }
    }
}

fn route_views(mut routes: Vec<RouteConfig>) -> Vec<RouteView> {
    sort_routes(&mut routes);

    routes
        .into_iter()
        .map(|route| {
            let header_operations = route.policies.request_headers.add.len()
                + route.policies.request_headers.remove.len()
                + route.policies.response_headers.add.len()
                + route.policies.response_headers.remove.len();
            let rate_limit = route.policies.rate_limit.as_ref().map_or_else(
                || "Off".to_owned(),
                |limit| format!("{} / {}s", limit.requests, limit.window_seconds),
            );
            let body_limit = route
                .policies
                .max_request_body_bytes
                .map_or_else(|| "Off".to_owned(), format_bytes);

            RouteView {
                name: route.name,
                host: route.host.unwrap_or_else(|| "*".to_owned()),
                path: route.path,
                methods: if route.methods.is_empty() {
                    "ANY".to_owned()
                } else {
                    route.methods.join(", ")
                },
                upstream: route.upstream,
                priority: route.priority,
                header_operations,
                rate_limit,
                body_limit,
            }
        })
        .collect()
}

fn health_views(endpoints: Vec<EndpointHealth>) -> Vec<EndpointHealthView> {
    endpoints
        .into_iter()
        .map(|endpoint| EndpointHealthView {
            upstream: endpoint.upstream,
            endpoint: endpoint.endpoint,
            status: if endpoint.healthy {
                "Healthy"
            } else {
                "Unhealthy"
            },
            status_class: if endpoint.healthy {
                "health-healthy"
            } else {
                "health-unhealthy"
            },
        })
        .collect()
}

fn format_bytes(bytes: usize) -> String {
    const KIB: usize = 1024;
    const MIB: usize = 1024 * KIB;

    if bytes >= MIB && bytes % MIB == 0 {
        format!("{} MiB", bytes / MIB)
    } else if bytes >= KIB && bytes % KIB == 0 {
        format!("{} KiB", bytes / KIB)
    } else {
        format!("{bytes} B")
    }
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
        http::{Method, Request, StatusCode, header},
    };
    use lungyam_core::{
        config::{AdminConfig, Config, RouteConfig, RoutePolicies, ServerConfig, UpstreamConfig},
        runtime::RuntimeStatus,
    };
    use tower::ServiceExt;

    use super::{
        DashboardTemplate, RoutesTemplate, format_bytes, format_uptime, health_views, route_views,
        router_with_status,
    };

    #[test]
    fn dashboard_template_renders_runtime_summary_health_and_htmx_contract() {
        let status = RuntimeStatus::from_config(&test_config());
        status.set_endpoint_health("api", "127.0.0.1:3001", false);
        let snapshot = status.snapshot();
        let active = snapshot.active_config;
        let template = DashboardTemplate {
            overview_active: true,
            routes_active: false,
            proxy_listen: active.proxy_listen,
            admin_listen: active.admin_listen,
            admin_mode: if active.admin_read_only {
                "Read-only".to_owned()
            } else {
                "Writes enabled".to_owned()
            },
            route_count: active.route_count,
            upstream_count: active.upstream_count,
            endpoint_count: active.endpoint_count,
            uptime: format_uptime(snapshot.uptime_seconds),
            endpoint_health: health_views(snapshot.endpoint_health),
        };
        let html = template.render().expect("dashboard should render");

        assert!(html.contains("Lungyam Admin"));
        assert!(html.contains("0.0.0.0:8080"));
        assert!(html.contains("127.0.0.1:9090"));
        assert!(html.contains(">1<"));
        assert!(html.contains(">2<"));
        assert!(html.contains("Uptime"));
        assert!(html.contains("127.0.0.1:3001"));
        assert!(html.contains("Unhealthy"));
        assert!(html.contains("/admin/assets/htmx.min.js"));
        assert!(html.contains("hx-get=\"/admin/fragments/upstream-health\""));
        assert!(html.contains("hx-trigger=\"every 5s\""));
        assert!(html.contains("href=\"/admin/routes\""));
    }

    #[test]
    fn routes_template_renders_effective_route_configuration() {
        let routes = route_views(test_config().routes);
        let template = RoutesTemplate {
            overview_active: false,
            routes_active: true,
            route_count: routes.len(),
            routes,
        };
        let html = template.render().expect("routes page should render");

        assert!(html.contains("Routes"));
        assert!(html.contains("api.test"));
        assert!(html.contains("GET, POST"));
        assert!(html.contains("/api"));
        assert!(html.contains("api"));
        assert!(html.contains("1 MiB"));
        assert!(html.contains("/admin/routes/new"));
    }

    #[test]
    fn route_views_follow_proxy_evaluation_order() {
        let mut low = test_config().routes.remove(0);
        low.name = "low".to_owned();
        low.priority = 10;
        low.path = "/api/long".to_owned();

        let mut high = low.clone();
        high.name = "high".to_owned();
        high.priority = 20;
        high.path = "/".to_owned();

        let mut specific = low.clone();
        specific.name = "specific".to_owned();
        specific.priority = 20;
        specific.path = "/api".to_owned();

        let views = route_views(vec![low, high, specific]);
        assert_eq!(views[0].name, "specific");
        assert_eq!(views[1].name, "high");
        assert_eq!(views[2].name, "low");
    }

    #[test]
    fn vendored_htmx_is_pinned_to_expected_version() {
        assert!(include_str!("../vendor/htmx.min.js").contains("version:\"2.0.10\""));
    }

    #[test]
    fn admin_routes_respond_over_http_service() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime");

        runtime.block_on(async {
            let status = Arc::new(RuntimeStatus::from_config(&test_config()));
            status.set_endpoint_health("api", "127.0.0.1:3001", false);
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

            let htmx = app
                .clone()
                .oneshot(
                    Request::builder()
                        .uri("/admin/assets/htmx.min.js")
                        .body(Body::empty())
                        .expect("htmx request"),
                )
                .await
                .expect("htmx response");
            assert_eq!(htmx.status(), StatusCode::OK);
            assert_eq!(
                htmx.headers()
                    .get(header::CONTENT_TYPE)
                    .and_then(|value| value.to_str().ok()),
                Some("text/javascript; charset=utf-8")
            );

            let fragment = app
                .clone()
                .oneshot(
                    Request::builder()
                        .uri("/admin/fragments/upstream-health")
                        .body(Body::empty())
                        .expect("fragment request"),
                )
                .await
                .expect("fragment response");
            assert_eq!(fragment.status(), StatusCode::OK);

            let routes = app
                .clone()
                .oneshot(
                    Request::builder()
                        .uri("/admin/routes")
                        .body(Body::empty())
                        .expect("routes request"),
                )
                .await
                .expect("routes response");
            assert_eq!(routes.status(), StatusCode::OK);

            let new_route = app
                .clone()
                .oneshot(
                    Request::builder()
                        .uri("/admin/routes/new")
                        .body(Body::empty())
                        .expect("new route request"),
                )
                .await
                .expect("new route response");
            assert_eq!(new_route.status(), StatusCode::OK);

            let valid_candidate = app
                .clone()
                .oneshot(
                    Request::builder()
                        .method(Method::POST)
                        .uri("/admin/routes/validate")
                        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                        .body(Body::from(
                            "name=new-route&host=&path=%2Fnew&methods=GET%2C+POST&upstream=api&priority=10&rate_limit_requests=10&rate_limit_window_seconds=60&max_request_body_bytes=1024",
                        ))
                        .expect("valid route request"),
                )
                .await
                .expect("valid route response");
            assert_eq!(valid_candidate.status(), StatusCode::OK);

            let invalid_candidate = app
                .clone()
                .oneshot(
                    Request::builder()
                        .method(Method::POST)
                        .uri("/admin/routes/validate")
                        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                        .body(Body::from(
                            "name=bad-route&host=&path=missing-slash&methods=&upstream=api&priority=0&rate_limit_requests=&rate_limit_window_seconds=&max_request_body_bytes=",
                        ))
                        .expect("invalid route request"),
                )
                .await
                .expect("invalid route response");
            assert_eq!(invalid_candidate.status(), StatusCode::OK);

            let simulation = app
                .clone()
                .oneshot(
                    Request::builder()
                        .method(Method::POST)
                        .uri("/admin/routes/simulate")
                        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                        .body(Body::from(
                            "host=api.test%3A8443&path=%2Fapi%2Fusers&method=POST",
                        ))
                        .expect("route simulation request"),
                )
                .await
                .expect("route simulation response");
            assert_eq!(simulation.status(), StatusCode::OK);

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
    fn formats_runtime_values() {
        assert_eq!(format_uptime(8), "8s");
        assert_eq!(format_uptime(68), "1m 8s");
        assert_eq!(format_uptime(3_668), "1h 1m 8s");
        assert_eq!(format_bytes(64), "64 B");
        assert_eq!(format_bytes(1024), "1 KiB");
        assert_eq!(format_bytes(1_048_576), "1 MiB");
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
                read_only: true,
            },
            upstreams,
            routes: vec![RouteConfig {
                name: "api".to_owned(),
                host: Some("api.test".to_owned()),
                path: "/api".to_owned(),
                methods: vec!["GET".to_owned(), "POST".to_owned()],
                upstream: "api".to_owned(),
                priority: 100,
                policies: RoutePolicies {
                    max_request_body_bytes: Some(1_048_576),
                    ..RoutePolicies::default()
                },
            }],
        }
    }
}
