use std::{collections::BTreeMap, sync::Arc};

use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode, header},
};
use lungyam_admin::router_with_status;
use lungyam_core::{
    config::{AdminConfig, Config, ServerConfig, UpstreamConfig},
    runtime::RuntimeStatus,
};
use tower::ServiceExt;

#[test]
fn upstream_validation_works_in_read_only_mode_without_config_path() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("test runtime");

    runtime.block_on(async {
        let config = test_config();
        let app = router_with_status(Arc::new(RuntimeStatus::from_config(&config)));

        let response = app
            .clone()
            .oneshot(validation_request(
                "canary",
                "127.0.0.1:4000%0A127.0.0.1:4001",
                "2500",
                "10000",
                "15000",
                "7",
            ))
            .await
            .expect("validation response");
        assert_eq!(response.status(), StatusCode::OK);
        let html = response_text(response).await;
        assert!(html.contains("Upstream candidate is valid"));
        assert!(html.contains("canary"));
        assert!(html.contains("Restart required"));

        let invalid = app
            .oneshot(validation_request(
                "bad",
                "not-an-endpoint",
                "0",
                "",
                "",
                "5",
            ))
            .await
            .expect("invalid validation response");
        assert_eq!(invalid.status(), StatusCode::OK);
        let invalid_html = response_text(invalid).await;
        assert!(invalid_html.contains("Upstream candidate is invalid"));
    });
}

fn validation_request(
    upstream_name: &str,
    endpoints: &str,
    connect_timeout: &str,
    read_timeout: &str,
    write_timeout: &str,
    health_check_interval: &str,
) -> Request<Body> {
    let body = format!(
        "operation=validate-upstream&upstream_name={upstream_name}&endpoints={endpoints}&connect_timeout={connect_timeout}&read_timeout={read_timeout}&write_timeout={write_timeout}&health_check_interval={health_check_interval}&name=_upstream_candidate&path=%2F&upstream=_unused"
    );
    Request::builder()
        .method("POST")
        .uri("/admin/routes/stage")
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(Body::from(body))
        .expect("validation request")
}

async fn response_text(response: axum::response::Response) -> String {
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read response body");
    String::from_utf8(bytes.to_vec()).expect("utf-8 response")
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
            read_only: true,
        },
        upstreams,
        routes: Vec::new(),
    }
}
