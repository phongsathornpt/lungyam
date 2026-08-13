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
use lungyam_admin::router_with_status_and_config_path;
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
fn route_staging_requires_csrf_and_keeps_active_config_unchanged() {
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
                    .expect("new route request"),
            )
            .await
            .expect("new route response");
        assert_eq!(page.status(), StatusCode::OK);
        let page_html = response_text(page).await;
        let csrf_token = hidden_value(&page_html, "csrf_token").expect("csrf token field");
        assert_eq!(csrf_token.len(), 64);
        assert!(page_html.contains("Stage pending revision"));

        let rejected = app
            .clone()
            .oneshot(stage_request("wrong-token", "rejected-route"))
            .await
            .expect("rejected stage response");
        assert_eq!(rejected.status(), StatusCode::FORBIDDEN);
        assert!(
            FileRevisionStore::beside_config(&config_path)
                .list()
                .expect("revision list after rejected stage")
                .is_empty()
        );

        let staged = app
            .oneshot(stage_request(&csrf_token, "staged-route"))
            .await
            .expect("stage response");
        assert_eq!(staged.status(), StatusCode::OK);
        let staged_html = response_text(staged).await;
        assert!(staged_html.contains("staged-route"));
        assert!(staged_html.contains("#000001"));

        assert_eq!(
            active_store.load().expect("reload active config"),
            config,
            "staging must not replace the active config file"
        );

        let revisions = FileRevisionStore::beside_config(&config_path);
        let history = revisions.list().expect("revision history");
        assert_eq!(history.len(), 1);
        let snapshot = revisions
            .load(history[0].revision)
            .expect("load staged revision");
        assert!(
            snapshot
                .config
                .routes
                .iter()
                .any(|route| route.name == "staged-route")
        );

        let state = FileRevisionStateStore::new(revisions.root())
            .load()
            .expect("revision state");
        assert_eq!(state.active_revision, None);
        assert_eq!(state.pending_revision, Some(history[0].revision));
    });

    fs::remove_dir_all(directory).expect("cleanup test directory");
}

#[test]
fn read_only_admin_does_not_offer_or_accept_route_staging() {
    let directory = test_directory();
    fs::create_dir_all(&directory).expect("create test directory");
    let config_path = directory.join("lungyam.yaml");
    let config = test_config(true);
    FileConfigStore::new(&config_path)
        .save(&config)
        .expect("save active config");

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
                    .expect("new route request"),
            )
            .await
            .expect("new route response");
        assert_eq!(page.status(), StatusCode::OK);
        let page_html = response_text(page).await;
        let csrf_token = hidden_value(&page_html, "csrf_token").expect("csrf token field");
        assert!(!page_html.contains("Stage pending revision"));
        assert!(page_html.contains("Admin writes are disabled"));

        let rejected = app
            .oneshot(stage_request(&csrf_token, "blocked-route"))
            .await
            .expect("read-only stage response");
        assert_eq!(rejected.status(), StatusCode::FORBIDDEN);
        assert!(
            FileRevisionStore::beside_config(&config_path)
                .list()
                .expect("revision list")
                .is_empty()
        );
    });

    fs::remove_dir_all(directory).expect("cleanup test directory");
}

fn stage_request(csrf_token: &str, route_name: &str) -> Request<Body> {
    let body = format!(
        "csrf_token={csrf_token}&name={route_name}&host=&path=%2Fstaged&methods=GET&upstream=api&priority=10&request_add_headers=&request_remove_headers=&response_add_headers=&response_remove_headers=&rate_limit_requests=&rate_limit_window_seconds=&max_request_body_bytes="
    );
    Request::builder()
        .method("POST")
        .uri("/admin/routes/stage")
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(Body::from(body))
        .expect("stage request")
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

fn test_directory() -> PathBuf {
    let sequence = TEST_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "lungyam-admin-route-staging-{}-{sequence}",
        std::process::id()
    ))
}

fn test_config(read_only: bool) -> Config {
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
            read_only,
        },
        upstreams,
        routes: Vec::new(),
    }
}
