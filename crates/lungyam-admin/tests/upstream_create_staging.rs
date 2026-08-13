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
fn upstream_create_staging_requires_csrf_and_keeps_active_config_unchanged() {
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
        let csrf_token = hidden_value(&page_html, "csrf_token").expect("csrf token field");
        assert!(page_html.contains("Stage upstream create"));
        assert!(page_html.contains("stage-upstream-create"));

        let rejected = app
            .clone()
            .oneshot(stage_request("wrong-token", "", "canary"))
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
            .oneshot(stage_request(&csrf_token, "", "canary"))
            .await
            .expect("stage response");
        assert_eq!(staged.status(), StatusCode::OK);
        let staged_html = response_text(staged).await;
        assert!(staged_html.contains("canary"));
        assert!(staged_html.contains("#000001"));
        assert!(staged_html.contains("active proxy configuration was not changed"));
        assert!(staged_html.contains("Restart required"));

        assert_eq!(
            active_store.load().expect("reload active config"),
            config,
            "upstream staging must not replace the active config file"
        );

        let revisions = FileRevisionStore::beside_config(&config_path);
        let history = revisions.list().expect("revision history");
        assert_eq!(history.len(), 1);
        let snapshot = revisions
            .load(history[0].revision)
            .expect("load staged revision");
        let canary = &snapshot.config.upstreams["canary"];
        assert_eq!(
            canary.endpoints,
            vec!["127.0.0.1:4000".to_owned(), "127.0.0.1:4001".to_owned()]
        );
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

    fs::remove_dir_all(directory).expect("cleanup test directory");
}

#[test]
fn read_only_admin_hides_and_rejects_upstream_create_staging() {
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
                    .expect("manage page request"),
            )
            .await
            .expect("manage page response");
        let page_html = response_text(page).await;
        let csrf_token = hidden_value(&page_html, "csrf_token").expect("csrf token field");
        assert!(!page_html.contains("Stage upstream create"));
        assert!(page_html.contains("Upstream validation remains available"));

        let rejected = app
            .oneshot(stage_request(&csrf_token, "", "canary"))
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

#[test]
fn upstream_create_staging_rejects_existing_identity() {
    let directory = test_directory();
    fs::create_dir_all(&directory).expect("create test directory");
    let config_path = directory.join("lungyam.yaml");
    let config = test_config(false);
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
                    .expect("manage page request"),
            )
            .await
            .expect("manage page response");
        let csrf_token = hidden_value(&response_text(page).await, "csrf_token").expect("csrf token");

        let rejected = app
            .oneshot(stage_request(&csrf_token, "api", "api"))
            .await
            .expect("existing identity response");
        assert_eq!(rejected.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert!(
            FileRevisionStore::beside_config(&config_path)
                .list()
                .expect("revision list")
                .is_empty()
        );
    });

    fs::remove_dir_all(directory).expect("cleanup test directory");
}

fn stage_request(csrf_token: &str, original_name: &str, upstream_name: &str) -> Request<Body> {
    let body = format!(
        "operation=stage-upstream-create&csrf_token={csrf_token}&upstream_original_name={original_name}&upstream_name={upstream_name}&endpoints=127.0.0.1%3A4000%0A127.0.0.1%3A4001&connect_timeout=2500&read_timeout=10000&write_timeout=15000&health_check_interval=7&name=_upstream_candidate&path=%2F&upstream=_unused"
    );
    Request::builder()
        .method("POST")
        .uri("/admin/routes/stage")
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(Body::from(body))
        .expect("upstream stage request")
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
        "lungyam-admin-upstream-create-staging-{}-{sequence}",
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
