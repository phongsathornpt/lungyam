use std::{collections::BTreeMap, sync::Arc};

use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use lungyam_admin::router_with_status;
use lungyam_core::{
    config::{AdminConfig, Config, ServerConfig, UpstreamConfig},
    runtime::RuntimeStatus,
};
use tower::ServiceExt;

#[test]
fn manage_page_renders_upstream_update_form() {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime")
        .block_on(async {
            let config = test_config();
            let app = router_with_status(Arc::new(RuntimeStatus::from_config(&config)));
            let response = app
                .oneshot(
                    Request::builder()
                        .uri("/admin/routes/new")
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);

            let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
            let html = String::from_utf8(bytes.to_vec()).unwrap();
            assert!(html.contains("Stage upstream updates"));
            assert!(html.contains("name=\"operation\" type=\"hidden\" value=\"update\""));
            assert!(html.contains("name=\"original_name\" type=\"hidden\" value=\"api\""));
        });
}

fn test_config() -> Config {
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
            read_only: false,
        },
        upstreams,
        routes: Vec::new(),
    }
}
