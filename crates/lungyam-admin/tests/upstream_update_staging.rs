use std::{collections::BTreeMap, fs, sync::Arc};

use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode, header},
};
use lungyam_admin::router_with_status_and_config_path;
use lungyam_core::{
    config::{AdminConfig, Config, ServerConfig, UpstreamConfig},
    revision::FileRevisionStore,
    runtime::RuntimeStatus,
    store::{ConfigStore, FileConfigStore},
};
use tower::ServiceExt;

#[test]
fn upstream_update_stages_pending_revision() {
    let directory = std::env::temp_dir().join(format!("lungyam-upstream-update-{}", std::process::id()));
    let _ = fs::remove_dir_all(&directory);
    fs::create_dir_all(&directory).expect("create test directory");
    let config_path = directory.join("lungyam.yaml");
    let config = test_config();
    FileConfigStore::new(&config_path).save(&config).expect("save config");

    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime")
        .block_on(async {
            let app = router_with_status_and_config_path(
                Arc::new(RuntimeStatus::from_config(&config)),
                config_path.clone(),
            );
            let page = app.clone().oneshot(Request::builder().uri("/admin/routes/new").body(Body::empty()).unwrap()).await.unwrap();
            let html = body(page).await;
            let token = hidden_value(&html, "csrf_token").expect("csrf token");
            assert!(html.contains("Stage upstream updates"));

            let form = format!("operation=update&original_name=api&csrf_token={token}&upstream_name=api&endpoints=127.0.0.1%3A4000%0A127.0.0.1%3A4001&connect_timeout=2500&read_timeout=11000&write_timeout=12000&health_check_interval=7");
            let response = app.oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/admin/upstreams/stage-create")
                    .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                    .body(Body::from(form))
                    .unwrap(),
            ).await.unwrap();
            assert_eq!(response.status(), StatusCode::OK);

            let history = FileRevisionStore::beside_config(&config_path).list().unwrap();
            let snapshot = FileRevisionStore::beside_config(&config_path).load(history[0].revision).unwrap();
            assert_eq!(snapshot.config.upstreams["api"].endpoints[0], "127.0.0.1:4000");
            assert_eq!(FileConfigStore::new(&config_path).load().unwrap(), config);
        });

    fs::remove_dir_all(directory).expect("cleanup");
}

async fn body(response: axum::response::Response) -> String {
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    String::from_utf8(bytes.to_vec()).unwrap()
}

fn hidden_value(html: &str, name: &str) -> Option<String> {
    let marker = format!("name=\"{name}\" type=\"hidden\" value=\"");
    let start = html.find(&marker)? + marker.len();
    let end = html[start..].find('"')? + start;
    Some(html[start..end].to_owned())
}

fn test_config() -> Config {
    let mut upstreams = BTreeMap::new();
    upstreams.insert("api".to_owned(), UpstreamConfig {
        endpoints: vec!["127.0.0.1:3000".to_owned()],
        connect_timeout_ms: Some(2000),
        read_timeout_ms: Some(10000),
        write_timeout_ms: Some(10000),
        health_check_interval_seconds: 5,
    });
    Config {
        server: ServerConfig { listen: "127.0.0.1:8080".to_owned() },
        admin: AdminConfig { enabled: true, listen: "127.0.0.1:9090".to_owned(), read_only: false },
        upstreams,
        routes: Vec::new(),
    }
}
