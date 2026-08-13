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
    http::{Request, StatusCode},
};
use lungyam_admin::router_with_status_and_config_path;
use lungyam_core::{
    config::{AdminConfig, Config, RouteConfig, RoutePolicies, ServerConfig, UpstreamConfig},
    revision::FileRevisionStore,
    revision_state::{FileRevisionStateStore, RevisionState},
    runtime::RuntimeStatus,
};
use tower::ServiceExt;

static TEST_SEQUENCE: AtomicU64 = AtomicU64::new(1);

#[test]
fn revisions_page_and_diff_fragment_render_persisted_state() {
    let directory = test_directory();
    fs::create_dir_all(&directory).expect("create test directory");
    let config_path = directory.join("lungyam.yaml");
    let config = test_config();
    let revisions = FileRevisionStore::beside_config(&config_path);

    let first = revisions
        .create(
            &config,
            Some("admin".to_owned()),
            Some("initial".to_owned()),
        )
        .expect("create first revision");
    let mut changed = config.clone();
    changed.routes[0].priority = 50;
    let second = revisions
        .create(
            &changed,
            Some("admin".to_owned()),
            Some("priority".to_owned()),
        )
        .expect("create second revision");

    FileRevisionStateStore::new(revisions.root())
        .save(&RevisionState {
            active_revision: Some(first.revision),
            pending_revision: Some(second.revision),
        })
        .expect("save revision state");

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("test runtime");

    runtime.block_on(async {
        let status = Arc::new(RuntimeStatus::from_config(&config));
        let app = router_with_status_and_config_path(status, config_path);

        let page = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/admin/revisions")
                    .body(Body::empty())
                    .expect("revisions request"),
            )
            .await
            .expect("revisions response");
        assert_eq!(page.status(), StatusCode::OK);
        let page_html = response_text(page).await;
        assert!(page_html.contains("#000001"));
        assert!(page_html.contains("#000002"));
        assert!(page_html.contains("Active"));
        assert!(page_html.contains("Pending"));
        assert!(page_html.contains("hx-get=\"/admin/fragments/config-diff\""));

        let diff = app
            .oneshot(
                Request::builder()
                    .uri("/admin/fragments/config-diff?from=1&to=2")
                    .body(Body::empty())
                    .expect("diff request"),
            )
            .await
            .expect("diff response");
        assert_eq!(diff.status(), StatusCode::OK);
        let diff_html = response_text(diff).await;
        assert!(diff_html.contains("#000001"));
        assert!(diff_html.contains("#000002"));
        assert!(diff_html.contains("Route-level change"));
        assert!(diff_html.contains("api"));
    });

    fs::remove_dir_all(directory).expect("cleanup test directory");
}

async fn response_text(response: axum::response::Response) -> String {
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read response body");
    String::from_utf8(bytes.to_vec()).expect("response body is utf-8")
}

fn test_directory() -> PathBuf {
    let sequence = TEST_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "lungyam-admin-revisions-{}-{sequence}",
        std::process::id()
    ))
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
            read_only: true,
        },
        upstreams,
        routes: vec![RouteConfig {
            name: "api".to_owned(),
            host: None,
            path: "/api".to_owned(),
            methods: vec!["GET".to_owned()],
            upstream: "api".to_owned(),
            priority: 0,
            policies: RoutePolicies::default(),
        }],
    }
}
