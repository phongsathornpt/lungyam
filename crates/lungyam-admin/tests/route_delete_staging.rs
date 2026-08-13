use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode, header},
};
use lungyam_admin::router_with_status_and_config_path;
use lungyam_core::{
    config::{AdminConfig, Config, RouteConfig, RoutePolicies, ServerConfig, UpstreamConfig},
    revision::FileRevisionStore,
    runtime::RuntimeStatus,
    store::{ConfigStore, FileConfigStore},
};
use std::{collections::BTreeMap, fs, sync::Arc};
use tower::ServiceExt;

#[test]
fn route_delete_stages_pending_revision() {
    let dir = std::env::temp_dir().join(format!("lungyam-route-delete-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let path = dir.join("lungyam.yaml");
    let config = test_config();
    let store = FileConfigStore::new(&path);
    store.save(&config).unwrap();

    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let app = router_with_status_and_config_path(
                Arc::new(RuntimeStatus::from_config(&config)),
                path.clone(),
            );
            let page = app
                .clone()
                .oneshot(
                    Request::builder()
                        .uri("/admin/routes/new")
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            let html = text(page).await;
            let token = hidden_value(&html, "csrf_token").unwrap();
            assert!(html.contains("Stage delete"));

            let body = format!(
                "operation=delete&original_name=api-route&csrf_token={token}&name=api-route&path=%2Fapi&upstream=api"
            );
            let response = app
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri("/admin/routes/stage")
                        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                        .body(Body::from(body))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            assert_eq!(store.load().unwrap(), config);

            let revisions = FileRevisionStore::beside_config(&path);
            let history = revisions.list().unwrap();
            assert_eq!(history.len(), 1);
            assert!(
                revisions
                    .load(history[0].revision)
                    .unwrap()
                    .config
                    .routes
                    .is_empty()
            );
        });
    fs::remove_dir_all(dir).unwrap();
}

async fn text(response: axum::response::Response) -> String {
    String::from_utf8(
        to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap()
            .to_vec(),
    )
    .unwrap()
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
