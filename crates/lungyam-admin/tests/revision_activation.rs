use std::{
    collections::BTreeMap,
    fs,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode, header},
};
use lungyam_admin::router_with_status_and_config_path;
use lungyam_core::{
    config::{AdminConfig, Config, RouteConfig, RoutePolicies, ServerConfig, UpstreamConfig},
    lifecycle::FileConfigLifecycle,
    runtime::RuntimeStatus,
    store::{ConfigStore, FileConfigStore},
};
use tower::ServiceExt;

static TEST_SEQUENCE: AtomicU64 = AtomicU64::new(1);

#[test]
fn route_only_activation_applies_live_runtime_snapshot() {
    let (directory, config_path, config) = setup();
    let mut candidate = config.clone();
    candidate.routes[0].path = "/v2".to_owned();
    FileConfigLifecycle::new(&config_path)
        .stage(&candidate, Some("test".to_owned()), Some("route update".to_owned()))
        .expect("stage route-only revision");

    let runtime_status = Arc::new(RuntimeStatus::from_config(&config));
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime");

    runtime.block_on(async {
        let app = router_with_status_and_config_path(Arc::clone(&runtime_status), config_path.clone());
        let token = csrf_from_revisions(&app).await;
        let response = app.oneshot(activate_request(&token)).await.expect("activation response");
        assert_eq!(response.status(), StatusCode::OK);
        let html = response_text(response).await;
        assert!(html.contains("activated and applied live"));
        assert_eq!(runtime_status.config().routes[0].path, "/v2");
        assert_eq!(FileConfigStore::new(&config_path).load().expect("active config"), candidate);
    });

    fs::remove_dir_all(directory).expect("cleanup");
}

#[test]
fn structural_activation_updates_disk_but_keeps_runtime_snapshot() {
    let (directory, config_path, config) = setup();
    let mut candidate = config.clone();
    candidate
        .upstreams
        .get_mut("api")
        .expect("api upstream")
        .endpoints
        .push("127.0.0.1:3001".to_owned());
    FileConfigLifecycle::new(&config_path)
        .stage(&candidate, Some("test".to_owned()), Some("upstream update".to_owned()))
        .expect("stage structural revision");

    let runtime_status = Arc::new(RuntimeStatus::from_config(&config));
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime");

    runtime.block_on(async {
        let app = router_with_status_and_config_path(Arc::clone(&runtime_status), config_path.clone());
        let token = csrf_from_revisions(&app).await;
        let response = app.oneshot(activate_request(&token)).await.expect("activation response");
        assert_eq!(response.status(), StatusCode::OK);
        let html = response_text(response).await;
        assert!(html.contains("restart required"));
        assert_eq!(runtime_status.config(), config);
        assert_eq!(FileConfigStore::new(&config_path).load().expect("active config"), candidate);
    });

    fs::remove_dir_all(directory).expect("cleanup");
}

fn setup() -> (std::path::PathBuf, std::path::PathBuf, Config) {
    let sequence = TEST_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let directory = std::env::temp_dir().join(format!(
        "lungyam-revision-activation-{}-{sequence}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&directory);
    fs::create_dir_all(&directory).expect("create test directory");
    let config_path = directory.join("lungyam.yaml");
    let config = test_config();
    FileConfigStore::new(&config_path)
        .save(&config)
        .expect("save active config");
    (directory, config_path, config)
}

async fn csrf_from_revisions(app: &axum::Router) -> String {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/admin/revisions")
                .body(Body::empty())
                .expect("revisions request"),
        )
        .await
        .expect("revisions response");
    assert_eq!(response.status(), StatusCode::OK);
    let html = response_text(response).await;
    hidden_value(&html, "csrf_token").expect("csrf token")
}

fn activate_request(csrf_token: &str) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri("/admin/revisions/activate")
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(Body::from(format!("csrf_token={csrf_token}")))
        .expect("activate request")
}

async fn response_text(response: axum::response::Response) -> String {
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read response body");
    String::from_utf8(bytes.to_vec()).expect("response body is utf-8")
}

fn hidden_value(html: &str, name: &str) -> Option<String> {
    let marker = format!("name=\"{name}\" type=\"hidden\" value=\"");
    let start = html.find(&marker)? + marker.len();
    let end = html[start..].find('"')? + start;
    Some(html[start..end].to_owned())
}

fn test_config() -> Config {
    let mut upstreams = BTreeMap::new();
    upstreams.insert(
        "api".to_owned(),
        UpstreamConfig {
            endpoints: vec!["127.0.0.1:3000".to_owned()],
            connect_timeout_ms: None,
            read_timeout_ms: None,
            write_timeout_ms: None,
            health_check_interval_seconds: 5,
        },
    );
    Config {
        server: ServerConfig {
            listen: "127.0.0.1:8080".to_owned(),
        },
        admin: AdminConfig {
            enabled: true,
            listen: "127.0.0.1:9090".to_owned(),
            read_only: false,
        },
        upstreams,
        routes: vec![RouteConfig {
            name: "api-route".to_owned(),
            host: None,
            path: "/api".to_owned(),
            methods: vec!["GET".to_owned()],
            upstream: "api".to_owned(),
            priority: 0,
            policies: RoutePolicies::default(),
        }],
    }
}
