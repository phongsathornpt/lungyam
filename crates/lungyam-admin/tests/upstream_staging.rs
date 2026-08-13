use std::{
    collections::BTreeMap,
    fs,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode, header},
};
use lungyam_admin::{router_with_status, router_with_status_and_config_path};
use lungyam_core::{
    config::{AdminConfig, Config, ServerConfig, UpstreamConfig},
    revision::FileRevisionStore,
    revision_state::FileRevisionStateStore,
    runtime::RuntimeStatus,
    store::{ConfigStore, FileConfigStore},
};
use tower::ServiceExt;

static TEST_SEQUENCE: AtomicU64 = AtomicU64::new(1);

#[test]
fn upstream_candidate_validation_is_available_in_read_only_mode() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("test runtime");

    runtime.block_on(async {
        let config = test_config(true);
        let app = router_with_status(Arc::new(RuntimeStatus::from_config(&config)));

        let response = app
            .oneshot(upstream_request(
                "validate-upstream",
                "",
                "canary",
                "127.0.0.1:4000%0A127.0.0.1:4001",
                "2500",
                "10000",
                "15000",
                "7",
                "",
            ))
            .await
            .expect("validation response");
        assert_eq!(response.status(), StatusCode::OK);
        let html = response_text(response).await;
        assert!(html.contains("Upstream candidate is valid"));
        assert!(html.contains("canary"));
        assert!(html.contains("Restart required"));
    });
}

#[test]
fn upstream_create_staging_keeps_active_config_unchanged() {
    let directory = test_directory();
    fs::create_dir_all(&directory).expect("create test directory");
    let config_path = directory.join("lungyam.yaml");
    let config = test_config(false);
    let active_store = FileConfigStore::new(&config_path);
    active_store.save(&config).expect("save active config");

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("test runtime");

    runtime.block_on(async {
        let status = Arc::new(RuntimeStatus::from_config(&config));
        let app = router_with_status_and_config_path(status, config_path.clone());
        let csrf = csrf_from_manage_page(app.clone()).await;

        let staged = app
            .oneshot(upstream_request(
                "stage-upstream",
                "",
                "canary",
                "127.0.0.1:4000%0A127.0.0.1:4001",
                "2500",
                "10000",
                "15000",
                "7",
                &csrf,
            ))
            .await
            .expect("stage response");
        assert_eq!(staged.status(), StatusCode::OK);
        let html = response_text(staged).await;
        assert!(html.contains("Upstream canary staged"));
        assert!(html.contains("#000001"));
        assert!(html.contains("Active proxy configuration is unchanged"));
        assert!(html.contains("Restart required"));

        assert_eq!(active_store.load().expect("reload active config"), config);

        let revisions = FileRevisionStore::beside_config(&config_path);
        let history = revisions.list().expect("revision history");
        assert_eq!(history.len(), 1);
        let snapshot = revisions
            .load(history[0].revision)
            .expect("load staged revision");
        let canary = &snapshot.config.upstreams["canary"];
        assert_eq!(canary.endpoints.len(), 2);
        assert_eq!(canary.connect_timeout_ms, Some(2500));
        assert_eq!(canary.read_timeout_ms, Some(10000));
        assert_eq!(canary.write_timeout_ms, Some(15000));
        assert_eq!(canary.health_check_interval_seconds, 7);

        let state = FileRevisionStateStore::new(revisions.root())
            .load()
            .expect("revision state");
        assert_eq!(state.active_revision, None);
        assert_eq!(state.pending_revision, Some(history[0].revision));
    });

    fs::remove_dir_all(directory).expect("cleanup");
}

#[test]
fn existing_upstream_form_stages_endpoint_and_timeout_updates() {
    let directory = test_directory();
    fs::create_dir_all(&directory).expect("create test directory");
    let config_path = directory.join("lungyam.yaml");
    let config = test_config(false);
    let active_store = FileConfigStore::new(&config_path);
    active_store.save(&config).expect("save active config");

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("test runtime");

    runtime.block_on(async {
        let status = Arc::new(RuntimeStatus::from_config(&config));
        let app = router_with_status_and_config_path(status, config_path.clone());
        let page = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/admin/routes/new")
                    .body(Body::empty())
                    .expect("manage page request"),
            )
            .await
            .expect("manage page response");
        assert_eq!(page.status(), StatusCode::OK);
        let page_html = response_text(page).await;
        let csrf = hidden_value(&page_html, "csrf_token").expect("csrf token");
        assert!(page_html.contains("Edit api"));
        assert!(page_html.contains("2000 ms"));
        assert!(page_html.contains("10000 ms"));
        assert!(page_html.contains("5 s"));
        assert!(page_html.contains("Validate update"));

        let staged = app
            .oneshot(upstream_request(
                "stage-upstream",
                "api",
                "api",
                "127.0.0.1:3100%0A127.0.0.1:3101",
                "3000",
                "12000",
                "16000",
                "9",
                &csrf,
            ))
            .await
            .expect("stage update response");
        assert_eq!(staged.status(), StatusCode::OK);

        assert_eq!(active_store.load().expect("reload active config"), config);
        let revisions = FileRevisionStore::beside_config(&config_path);
        let history = revisions.list().expect("revision history");
        assert_eq!(history.len(), 1);
        let snapshot = revisions
            .load(history[0].revision)
            .expect("load staged update");
        let upstream = &snapshot.config.upstreams["api"];
        assert_eq!(
            upstream.endpoints,
            vec![
                "127.0.0.1:3100".to_owned(),
                "127.0.0.1:3101".to_owned()
            ]
        );
        assert_eq!(upstream.connect_timeout_ms, Some(3000));
        assert_eq!(upstream.read_timeout_ms, Some(12000));
        assert_eq!(upstream.write_timeout_ms, Some(16000));
        assert_eq!(upstream.health_check_interval_seconds, 9);
    });

    fs::remove_dir_all(directory).expect("cleanup");
}

async fn csrf_from_manage_page(app: axum::Router) -> String {
    let response = app
        .oneshot(
            Request::builder()
                .uri("/admin/routes/new")
                .body(Body::empty())
                .expect("manage page request"),
        )
        .await
        .expect("manage page response");
    assert_eq!(response.status(), StatusCode::OK);
    hidden_value(&response_text(response).await, "csrf_token").expect("csrf token")
}

fn upstream_request(
    operation: &str,
    original_name: &str,
    upstream_name: &str,
    endpoints: &str,
    connect_timeout: &str,
    read_timeout: &str,
    write_timeout: &str,
    health_check_interval: &str,
    csrf_token: &str,
) -> Request<Body> {
    let body = format!(
        "operation={operation}&upstream_original_name={original_name}&upstream_name={upstream_name}&endpoints={endpoints}&connect_timeout={connect_timeout}&read_timeout={read_timeout}&write_timeout={write_timeout}&health_check_interval={health_check_interval}&csrf_token={csrf_token}&name=_upstream_candidate&path=%2F&upstream=_unused"
    );
    Request::builder()
        .method("POST")
        .uri("/admin/routes/stage")
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(Body::from(body))
        .expect("upstream form request")
}

async fn response_text(response: axum::response::Response) -> String {
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read response body");
    String::from_utf8(bytes.to_vec()).expect("utf-8 response")
}

fn hidden_value(html: &str, name: &str) -> Option<String> {
    let marker = format!("name=\"{name}\" type=\"hidden\" value=\"");
    let start = html.find(&marker)? + marker.len();
    let end = html[start..].find('"')? + start;
    Some(html[start..end].to_owned())
}

fn test_directory() -> PathBuf {
    let sequence = TEST_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "lungyam-admin-upstream-staging-{}-{sequence}",
        std::process::id()
    ))
}

fn test_config(read_only: bool) -> Config {
    let mut upstreams = BTreeMap::new();
    upstreams.insert(
        "api".to_owned(),
        UpstreamConfig {
            endpoints: vec!["127.0.0.1:3000".to_owned()],
            connect_timeout_ms: Some(2000),
            read_timeout_ms: Some(10000),
            write_timeout_ms: Some(10000),
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
            read_only,
        },
        upstreams,
        routes: Vec::new(),
    }
}
