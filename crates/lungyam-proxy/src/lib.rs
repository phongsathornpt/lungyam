//! Native Pingora-based proxy runtime for Lungyam.

use std::{
    collections::BTreeMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use async_trait::async_trait;
use bytes::Bytes;
use http::{HeaderName, HeaderValue};
use lungyam_core::{
    PROJECT_NAME,
    config::{Config, HeaderTransform, RouteConfig},
};
use pingora::{
    http::{RequestHeader, ResponseHeader},
    lb::{LoadBalancer, health_check::TcpHealthCheck, selection::RoundRobin},
    prelude::*,
    services::background::GenBackgroundService,
};

/// Per-request state shared by Pingora filters.
#[derive(Debug)]
pub struct RequestContext {
    route_index: Option<usize>,
    request_id: String,
    started: Instant,
    connect_failures: usize,
}

#[derive(Debug)]
struct WindowCounter {
    started: Instant,
    count: u64,
}

type UpstreamCluster = Arc<LoadBalancer<RoundRobin>>;
type UpstreamHealthService = GenBackgroundService<LoadBalancer<RoundRobin>>;

/// Configuration-driven Lungyam gateway.
pub struct Gateway {
    config: Config,
    clusters: BTreeMap<String, UpstreamCluster>,
    rate_limits: Mutex<BTreeMap<String, WindowCounter>>,
    request_sequence: AtomicU64,
}

impl Gateway {
    /// Creates a gateway and pre-orders routes by priority and specificity.
    #[must_use]
    pub fn new(mut config: Config, clusters: BTreeMap<String, UpstreamCluster>) -> Self {
        config.routes.sort_by(|left, right| {
            right
                .priority
                .cmp(&left.priority)
                .then_with(|| right.path.len().cmp(&left.path.len()))
        });

        Self {
            config,
            clusters,
            rate_limits: Mutex::new(BTreeMap::new()),
            request_sequence: AtomicU64::new(1),
        }
    }

    fn route_index(&self, session: &Session) -> Option<usize> {
        let request = session.req_header();
        let host = request
            .headers
            .get("host")
            .and_then(|value| value.to_str().ok());
        let path = request.uri.path();
        let method = request.method.as_str();

        self.config
            .routes
            .iter()
            .position(|route| route_matches(route, host, path, method))
    }

    fn route<'a>(&'a self, ctx: &RequestContext) -> &'a RouteConfig {
        &self.config.routes[ctx
            .route_index
            .expect("request filter must select a route before proxying")]
    }

    fn allow_request(&self, route: &RouteConfig) -> bool {
        let Some(limit) = &route.policies.rate_limit else {
            return true;
        };

        let now = Instant::now();
        let mut states = self
            .rate_limits
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let state = states.entry(route.name.clone()).or_insert(WindowCounter {
            started: now,
            count: 0,
        });

        if now.duration_since(state.started) >= Duration::from_secs(limit.window_seconds) {
            state.started = now;
            state.count = 0;
        }

        if state.count >= limit.requests {
            return false;
        }

        state.count += 1;
        true
    }

    fn can_retry_connect(&self, ctx: &RequestContext) -> bool {
        let route = self.route(ctx);
        self.config
            .upstreams
            .get(&route.upstream)
            .is_some_and(|upstream| ctx.connect_failures < upstream.endpoints.len())
    }
}

#[async_trait]
impl ProxyHttp for Gateway {
    type CTX = RequestContext;

    fn new_ctx(&self) -> Self::CTX {
        let sequence = self.request_sequence.fetch_add(1, Ordering::Relaxed);
        RequestContext {
            route_index: None,
            request_id: format!("ly-{sequence}"),
            started: Instant::now(),
            connect_failures: 0,
        }
    }

    async fn request_filter(&self, session: &mut Session, ctx: &mut Self::CTX) -> Result<bool> {
        if session.req_header().uri.path() == "/health" {
            session
                .respond_error_with_body(200, Bytes::from_static(b"ok\n"))
                .await?;
            return Ok(true);
        }

        let Some(route_index) = self.route_index(session) else {
            session
                .respond_error_with_body(404, Bytes::from_static(b"route not found\n"))
                .await?;
            return Ok(true);
        };
        ctx.route_index = Some(route_index);

        let route = self.route(ctx);
        if let Some(limit) = route.policies.max_request_body_bytes {
            let content_length = session
                .req_header()
                .headers
                .get("content-length")
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.parse::<usize>().ok());
            if content_length.is_some_and(|length| length > limit) {
                session
                    .respond_error_with_body(413, Bytes::from_static(b"request body too large\n"))
                    .await?;
                return Ok(true);
            }
        }

        if !self.allow_request(route) {
            session
                .respond_error_with_body(429, Bytes::from_static(b"rate limit exceeded\n"))
                .await?;
            return Ok(true);
        }

        Ok(false)
    }

    async fn upstream_peer(
        &self,
        _session: &mut Session,
        ctx: &mut Self::CTX,
    ) -> Result<Box<HttpPeer>> {
        let route = self.route(ctx);
        let upstream = self
            .config
            .upstreams
            .get(&route.upstream)
            .expect("configuration validation guarantees upstream references");
        let cluster = self
            .clusters
            .get(&route.upstream)
            .expect("load balancer exists for every validated upstream");
        let Some(backend) = cluster.select(b"", 256) else {
            return Error::e_explain(
                ErrorType::ConnectNoRoute,
                format!("no healthy endpoints for upstream '{}'", route.upstream),
            );
        };

        let mut peer = Box::new(HttpPeer::new(backend, false, String::new()));
        peer.options.connection_timeout = upstream.connect_timeout_ms.map(Duration::from_millis);
        peer.options.read_timeout = upstream.read_timeout_ms.map(Duration::from_millis);
        peer.options.write_timeout = upstream.write_timeout_ms.map(Duration::from_millis);
        Ok(peer)
    }

    fn fail_to_connect(
        &self,
        _session: &mut Session,
        _peer: &HttpPeer,
        ctx: &mut Self::CTX,
        mut error: Box<Error>,
    ) -> Box<Error> {
        ctx.connect_failures += 1;
        let retry = self.can_retry_connect(ctx);
        error.set_retry(retry);

        if retry {
            let route = self.route(ctx);
            log::warn!(
                "request_id={} route={} upstream={} connect_failure={} retrying=true",
                ctx.request_id,
                route.name,
                route.upstream,
                ctx.connect_failures
            );
        }

        error
    }

    async fn upstream_request_filter(
        &self,
        _session: &mut Session,
        upstream_request: &mut RequestHeader,
        ctx: &mut Self::CTX,
    ) -> Result<()> {
        let route = self.route(ctx);
        apply_request_headers(upstream_request, &route.policies.request_headers);
        upstream_request
            .insert_header("x-request-id", ctx.request_id.as_str())
            .expect("generated request id is a valid header value");
        upstream_request
            .insert_header("x-lungyam-route", route.name.as_str())
            .expect("validated route name is a valid header value");
        Ok(())
    }

    async fn response_filter(
        &self,
        _session: &mut Session,
        response: &mut ResponseHeader,
        ctx: &mut Self::CTX,
    ) -> Result<()> {
        let route = self.route(ctx);
        apply_response_headers(response, &route.policies.response_headers);
        response
            .insert_header("x-request-id", ctx.request_id.as_str())
            .expect("generated request id is a valid header value");
        response.remove_header("alt-svc");
        Ok(())
    }

    async fn logging(
        &self,
        session: &mut Session,
        error: Option<&pingora::Error>,
        ctx: &mut Self::CTX,
    ) {
        let status = session
            .response_written()
            .map_or(0, |response| response.status.as_u16());
        let route = ctx
            .route_index
            .map_or("-", |index| self.config.routes[index].name.as_str());
        log::info!(
            "request_id={} route={} method={} path={} status={} latency_ms={} connect_failures={} error={}",
            ctx.request_id,
            route,
            session.req_header().method,
            session.req_header().uri,
            status,
            ctx.started.elapsed().as_millis(),
            ctx.connect_failures,
            error.is_some()
        );
    }
}

fn apply_request_headers(header: &mut RequestHeader, transform: &HeaderTransform) {
    for name in &transform.remove {
        header.remove_header(name.as_str());
    }
    for (name, value) in &transform.add {
        let name = HeaderName::from_bytes(name.as_bytes())
            .expect("configuration header name must be valid");
        let value = HeaderValue::from_str(value).expect("configuration header value must be valid");
        header
            .insert_header(name, value)
            .expect("configuration header transform must be valid");
    }
}

fn apply_response_headers(header: &mut ResponseHeader, transform: &HeaderTransform) {
    for name in &transform.remove {
        header.remove_header(name.as_str());
    }
    for (name, value) in &transform.add {
        let name = HeaderName::from_bytes(name.as_bytes())
            .expect("configuration header name must be valid");
        let value = HeaderValue::from_str(value).expect("configuration header value must be valid");
        header
            .insert_header(name, value)
            .expect("configuration header transform must be valid");
    }
}

fn route_matches(route: &RouteConfig, host: Option<&str>, path: &str, method: &str) -> bool {
    host_matches(route.host.as_deref(), host)
        && path_matches(&route.path, path)
        && (route.methods.is_empty()
            || route
                .methods
                .iter()
                .any(|allowed| allowed.eq_ignore_ascii_case(method)))
}

fn host_matches(expected: Option<&str>, actual: Option<&str>) -> bool {
    let Some(expected) = expected else {
        return true;
    };
    let Some(actual) = actual else {
        return false;
    };

    actual.eq_ignore_ascii_case(expected)
        || actual
            .strip_prefix(expected)
            .is_some_and(|suffix| suffix.starts_with(':'))
}

fn path_matches(route_path: &str, actual: &str) -> bool {
    if route_path == "/" || route_path == actual {
        return true;
    }

    actual
        .strip_prefix(route_path)
        .is_some_and(|suffix| suffix.starts_with('/'))
}

fn build_upstream_clusters(
    config: &Config,
) -> (
    BTreeMap<String, UpstreamCluster>,
    Vec<UpstreamHealthService>,
) {
    let mut clusters = BTreeMap::new();
    let mut health_services = Vec::with_capacity(config.upstreams.len());

    for (name, upstream) in &config.upstreams {
        let mut cluster =
            LoadBalancer::try_from_iter(upstream.endpoints.iter().map(String::as_str))
                .expect("configuration validation guarantees valid upstream endpoints");
        cluster.set_health_check(TcpHealthCheck::new());
        cluster.health_check_frequency =
            Some(Duration::from_secs(upstream.health_check_interval_seconds));

        let service_name = format!("upstream {name} health check");
        let health_service = background_service(&service_name, cluster);
        clusters.insert(name.clone(), health_service.task());
        health_services.push(health_service);
    }

    (clusters, health_services)
}

/// Starts the native Lungyam proxy and blocks until the server exits.
pub fn run(config: Config) {
    let listen = config.server.listen.clone();
    let (clusters, health_services) = build_upstream_clusters(&config);
    let mut server = Server::new(None).expect("failed to create Pingora server");
    server.bootstrap();

    let mut service = http_proxy_service(&server.configuration, Gateway::new(config, clusters));
    service.add_tcp(&listen);
    server.add_service(service);
    for health_service in health_services {
        server.add_service(health_service);
    }

    log::info!("{} listening on {}", runtime_banner(), listen);
    server.run_forever();
}

/// Returns the runtime banner used during bootstrap.
#[must_use]
pub fn runtime_banner() -> String {
    format!("{PROJECT_NAME} proxy")
}

#[cfg(test)]
mod tests {
    use super::{host_matches, path_matches};

    #[test]
    fn path_matching_respects_segment_boundaries() {
        assert!(path_matches("/api", "/api"));
        assert!(path_matches("/api", "/api/users"));
        assert!(!path_matches("/api", "/apiv2"));
    }

    #[test]
    fn host_matching_accepts_optional_port() {
        assert!(host_matches(
            Some("api.example.com"),
            Some("api.example.com")
        ));
        assert!(host_matches(
            Some("api.example.com"),
            Some("api.example.com:8080")
        ));
        assert!(!host_matches(
            Some("api.example.com"),
            Some("other.example.com")
        ));
    }
}
