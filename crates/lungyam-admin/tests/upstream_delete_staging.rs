use std::{collections::BTreeMap, fs, sync::Arc};

use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode, header},
};
use lungyam_admin::router_with_status_and_config_path;
use lungyam_core::{
    config::{
        AdminConfig, Config, HeaderTransform, RouteConfig, RoutePolicies, ServerConfig,
        UpstreamConfig,
    },
    revision::FileRevisionStore,
    runtime::RuntimeStatus,
    store::{ConfigStore, FileConfigStore},
};
use tower::ServiceExt;

#[test]
fn upstream_delete_rejects_referenced_pool_and_stages_unreferenced_pool() {
    let directory =
        std::env::temp_dir().join(format!("lungyam-upstream-delete-{}", std::process::id()));
    let _ = fs::remove_dir_all(&directory);
    fs::create_dir_all(&directory).expect("create test directory");
    let config_path = directory.join("lungyam.yaml");
    let config = test_config();
    let active_store = FileConfigStore::new(&config_path);
    active_store.save(&config).expect("save config");

    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime")
        .block_on(async {
            let app = router_with_status_and_config_path(
                Arc::new(RuntimeStatus::from_config(&config)),
                config_path.clone(),
            );
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
            let html = response_text(page).await;
            let token = hidden_value(&html, "csrf_token").expect("csrf token");

            let rejected = app
                .clone()
                .oneshot(delete_request(&token, "api"))
                .await
                .expect("referenced delete response");
            assert_eq!(rejected.status(), StatusCode::UNPROCESSABLE_ENTITY);
            assert!(
                FileRevisionStore::beside_config(&config_path)
                    .list()
                    .expect("revisions after rejected delete")
                    .is_empty()
            );

            let staged = app
                .oneshot(delete_request(&token, "canary"))
                .await
                .expect("unreferenced delete response");
            assert_eq!(staged.status(), StatusCode::OK);

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
            assert!(snapshot.config.upstreams.contains_key("api"));
            assert!(!snapshot.config.upstreams.contains_key("canary"));
        });

    fs::remove_dir_all(directory).expect("cleanup");
}

fn delete_request(csrf_token: &str, upstream_name: &str) -> Request<Body> {
    let body = format!("operation=delete&csrf_token={csrf_token}&upstream_name={upstream_name}");
    Request::builder()
        .method("POST")
        .uri("/admin/upstreams/stage-create")
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(Body::from(body))
        .expect("delete request")
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
    upstreams.insert(
        "canary".to_owned(),
        UpstreamConfig {
            endpoints: vec!["127.0.0.1:4000".to_owned()],
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
            policies: RoutePolicies {
                request_headers: HeaderTransform::default(),
                response_headers: HeaderTransform::default(),
                rate_limit: None,
                max_request_body_bytes: None,
            },
        }],
    }
}
