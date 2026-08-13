use axum::{
    Form,
    extract::State,
    http::StatusCode,
    response::{Html, IntoResponse, Response},
};
use lungyam_core::{
    config::{Config, UpstreamConfig},
    lifecycle::FileConfigLifecycle,
};
use serde::Deserialize;

use crate::{AdminState, security};

#[derive(Debug, Deserialize)]
pub(super) struct StageUpstreamCreateForm {
    #[serde(default)]
    csrf_token: String,
    upstream_name: String,
    endpoints: String,
    #[serde(default)]
    connect_timeout: String,
    #[serde(default)]
    read_timeout: String,
    #[serde(default)]
    write_timeout: String,
    health_check_interval: String,
}

pub(super) async fn stage_create(
    State(state): State<AdminState>,
    Form(form): Form<StageUpstreamCreateForm>,
) -> Response {
    let config = state.runtime.config();

    if config.admin.read_only {
        return denied("Admin is configured as read-only.");
    }
    if !security::writes_enabled(&config) {
        return denied(
            "Admin writes require a loopback listener until authentication is configured.",
        );
    }
    if !security::csrf_token().verify(&form.csrf_token) {
        return denied("CSRF token validation failed.");
    }

    let Some(config_path) = state.config_path.as_ref() else {
        return render_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "Config path is unavailable; staging is disabled for this admin router.",
        );
    };

    let candidate = match candidate_config(&config, &form) {
        Ok(candidate) => candidate,
        Err(message) => return render_error(StatusCode::UNPROCESSABLE_ENTITY, &message),
    };
    let upstream_name = form.upstream_name.trim();

    match FileConfigLifecycle::new(config_path).stage(
        &candidate,
        Some("admin-web".to_owned()),
        Some(format!("stage upstream create '{upstream_name}'")),
    ) {
        Ok(metadata) => render_success(metadata.revision),
        Err(error) => {
            log::error!("failed to stage upstream create: {error}");
            render_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "The pending revision could not be created.",
            )
        }
    }
}

fn candidate_config(config: &Config, form: &StageUpstreamCreateForm) -> Result<Config, String> {
    let upstream_name = form.upstream_name.trim();
    if upstream_name.is_empty() {
        return Err("upstream name must not be empty".to_owned());
    }
    if config.upstreams.contains_key(upstream_name) {
        return Err(format!("upstream '{upstream_name}' already exists"));
    }

    let endpoints = form
        .endpoints
        .split([',', '\n'])
        .map(str::trim)
        .filter(|endpoint| !endpoint.is_empty())
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    if endpoints.is_empty() {
        return Err("at least one upstream endpoint is required".to_owned());
    }

    let mut candidate = config.clone();
    candidate.upstreams.insert(
        upstream_name.to_owned(),
        UpstreamConfig {
            endpoints,
            connect_timeout_ms: parse_optional_duration(
                &form.connect_timeout,
                "connect timeout",
                "ms",
            )?,
            read_timeout_ms: parse_optional_duration(&form.read_timeout, "read timeout", "ms")?,
            write_timeout_ms: parse_optional_duration(&form.write_timeout, "write timeout", "ms")?,
            health_check_interval_seconds: parse_required_duration(
                &form.health_check_interval,
                "health-check interval",
                "s",
            )?,
        },
    );
    candidate.validate().map_err(|error| error.to_string())?;
    Ok(candidate)
}

fn parse_optional_duration(value: &str, label: &str, suffix: &str) -> Result<Option<u64>, String> {
    let value = value.trim();
    if value.is_empty() || value.eq_ignore_ascii_case("default") {
        return Ok(None);
    }
    let value = value.strip_suffix(suffix).map(str::trim).unwrap_or(value);
    let parsed = value
        .parse::<u64>()
        .map_err(|_| format!("{label} must be a valid number"))?;
    if parsed == 0 {
        return Err(format!("{label} must be greater than zero"));
    }
    Ok(Some(parsed))
}

fn parse_required_duration(value: &str, label: &str, suffix: &str) -> Result<u64, String> {
    let value = value.trim();
    let value = value.strip_suffix(suffix).map(str::trim).unwrap_or(value);
    let parsed = value
        .parse::<u64>()
        .map_err(|_| format!("{label} must be a valid number"))?;
    if parsed == 0 {
        return Err(format!("{label} must be greater than zero"));
    }
    Ok(parsed)
}

fn denied(message: &str) -> Response {
    render_error(StatusCode::FORBIDDEN, message)
}

fn render_success(revision: u64) -> Response {
    let html = format!(
        "<div class=\"validation-card validation-success\"><strong>Pending revision created</strong><p>Revision #{revision:06} is ready for review. The active proxy configuration was not changed.</p><p><strong>Restart required:</strong> upstream topology changes are not hot-reloadable yet.</p></div>"
    );
    (StatusCode::OK, Html(html)).into_response()
}

fn render_error(status: StatusCode, message: &str) -> Response {
    log::warn!("upstream create staging rejected: {message}");
    (
        status,
        Html(
            "<div class=\"validation-card validation-error\"><strong>Upstream was not staged</strong><p>Review the candidate validation and admin write settings.</p></div>"
                .to_owned(),
        ),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use std::{
        collections::BTreeMap,
        fs,
        path::PathBuf,
        sync::{
            Arc,
            atomic::{AtomicU64, Ordering},
        },
    };

    use axum::{Form, extract::State, http::StatusCode};
    use lungyam_core::{
        config::{AdminConfig, Config, ServerConfig, UpstreamConfig},
        revision::FileRevisionStore,
        revision_state::FileRevisionStateStore,
        runtime::RuntimeStatus,
        store::{ConfigStore, FileConfigStore},
    };

    use super::{StageUpstreamCreateForm, candidate_config, stage_create};
    use crate::{AdminState, security};

    static TEST_SEQUENCE: AtomicU64 = AtomicU64::new(1);

    #[test]
    fn candidate_create_rejects_duplicate_and_parses_values() {
        let config = test_config(false);
        let duplicate = form("api");
        assert!(candidate_config(&config, &duplicate).is_err());

        let candidate = candidate_config(&config, &form("canary")).expect("valid candidate");
        let canary = &candidate.upstreams["canary"];
        assert_eq!(canary.endpoints.len(), 2);
        assert_eq!(canary.connect_timeout_ms, Some(2500));
        assert_eq!(canary.read_timeout_ms, Some(10000));
        assert_eq!(canary.write_timeout_ms, Some(15000));
        assert_eq!(canary.health_check_interval_seconds, 7);
    }

    #[test]
    fn handler_stages_pending_revision_without_replacing_active_config() {
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
            let state = AdminState {
                runtime: Arc::new(RuntimeStatus::from_config(&config)),
                config_path: Some(config_path.clone()),
            };
            let mut request = form("canary");
            request.csrf_token = security::csrf_token().expose().to_owned();

            let response = stage_create(State(state), Form(request)).await;
            assert_eq!(response.status(), StatusCode::OK);

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
            let canary = &snapshot.config.upstreams["canary"];
            assert_eq!(canary.endpoints.len(), 2);
            assert_eq!(canary.connect_timeout_ms, Some(2500));
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
    fn handler_rejects_read_only_and_invalid_csrf_before_staging() {
        let directory = test_directory();
        fs::create_dir_all(&directory).expect("create test directory");
        let config_path = directory.join("lungyam.yaml");

        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime");

        runtime.block_on(async {
            let read_only = test_config(true);
            FileConfigStore::new(&config_path)
                .save(&read_only)
                .expect("save active config");
            let read_only_state = AdminState {
                runtime: Arc::new(RuntimeStatus::from_config(&read_only)),
                config_path: Some(config_path.clone()),
            };
            let mut request = form("canary");
            request.csrf_token = security::csrf_token().expose().to_owned();
            let response = stage_create(State(read_only_state), Form(request)).await;
            assert_eq!(response.status(), StatusCode::FORBIDDEN);

            let writable = test_config(false);
            let writable_state = AdminState {
                runtime: Arc::new(RuntimeStatus::from_config(&writable)),
                config_path: Some(config_path.clone()),
            };
            let mut request = form("canary");
            request.csrf_token = "wrong-token".to_owned();
            let response = stage_create(State(writable_state), Form(request)).await;
            assert_eq!(response.status(), StatusCode::FORBIDDEN);

            assert!(
                FileRevisionStore::beside_config(&config_path)
                    .list()
                    .expect("revision list")
                    .is_empty()
            );
        });

        fs::remove_dir_all(directory).expect("cleanup test directory");
    }

    fn form(upstream_name: &str) -> StageUpstreamCreateForm {
        StageUpstreamCreateForm {
            csrf_token: String::new(),
            upstream_name: upstream_name.to_owned(),
            endpoints: "127.0.0.1:4000\n127.0.0.1:4001".to_owned(),
            connect_timeout: "2500".to_owned(),
            read_timeout: "10000".to_owned(),
            write_timeout: "15000".to_owned(),
            health_check_interval: "7".to_owned(),
        }
    }

    fn test_directory() -> PathBuf {
        let sequence = TEST_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "lungyam-admin-upstream-stage-{}-{sequence}",
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
}
