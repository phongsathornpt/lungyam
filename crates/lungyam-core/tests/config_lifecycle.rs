use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

use lungyam_core::{
    config::Config,
    lifecycle::{ConfigLifecycleError, FileConfigLifecycle},
    store::{ConfigStore, FileConfigStore},
};

static TEST_SEQUENCE: AtomicU64 = AtomicU64::new(1);

const VALID: &str = r#"
server:
  listen: 127.0.0.1:8080
upstreams:
  api:
    endpoints: [127.0.0.1:3000]
routes:
  - name: api
    path: /api
    methods: [GET]
    upstream: api
"#;

#[test]
fn lifecycle_stages_activates_and_restores_validated_revisions() {
    let directory = test_directory();
    fs::create_dir_all(&directory).expect("create test directory");
    let config_path = directory.join("lungyam.yaml");
    let active = FileConfigStore::new(&config_path);
    let initial = Config::from_yaml(VALID).expect("valid fixture");
    active.save(&initial).expect("save initial config");
    let lifecycle = FileConfigLifecycle::new(&config_path);

    let mut route_change = initial.clone();
    route_change.routes[0].priority = 50;
    let first = lifecycle
        .stage(&route_change, Some("admin".into()), Some("priority".into()))
        .expect("stage route change");
    assert_eq!(active.load().expect("active before activation"), initial);
    assert_eq!(
        lifecycle.state_store().load().expect("staged state").pending_revision,
        Some(first.revision)
    );

    let activated = lifecycle.activate_pending().expect("activate route change");
    assert!(!activated.restart_required);
    assert_eq!(activated.diff.routes_changed, vec!["api"]);
    assert_eq!(active.load().expect("active route change"), route_change);

    let mut structural = route_change.clone();
    structural
        .upstreams
        .get_mut("api")
        .expect("api upstream")
        .read_timeout_ms = Some(5_000);
    let second = lifecycle
        .stage(&structural, None, Some("timeout".into()))
        .expect("stage structural change");
    let activated = lifecycle.activate_pending().expect("activate structural change");
    assert_eq!(activated.revision, second.revision);
    assert!(activated.restart_required);

    let restored = lifecycle
        .rollback_to(first.revision, Some("admin".into()), None)
        .expect("restore historical config");
    assert!(restored.revision > second.revision);
    assert_eq!(active.load().expect("restored active config"), route_change);
    let restored_snapshot = lifecycle
        .revision_store()
        .load(restored.revision)
        .expect("restored revision");
    assert_eq!(
        restored_snapshot.metadata.reason.as_deref(),
        Some("rollback to #000001")
    );
    let state = lifecycle.state_store().load().expect("final state");
    assert_eq!(state.active_revision, Some(restored.revision));
    assert_eq!(state.pending_revision, None);

    fs::remove_dir_all(directory).expect("cleanup");
}

#[test]
fn invalid_stage_and_missing_pending_do_not_change_active_config() {
    let directory = test_directory();
    fs::create_dir_all(&directory).expect("create test directory");
    let config_path = directory.join("lungyam.yaml");
    let active = FileConfigStore::new(&config_path);
    let initial = Config::from_yaml(VALID).expect("valid fixture");
    active.save(&initial).expect("save initial config");
    let lifecycle = FileConfigLifecycle::new(&config_path);

    let mut invalid = initial.clone();
    invalid.routes[0].path = "missing-slash".into();
    assert!(lifecycle.stage(&invalid, None, None).is_err());
    assert!(lifecycle.revision_store().list().expect("revision list").is_empty());
    assert_eq!(active.load().expect("unchanged active config"), initial);
    assert!(matches!(
        lifecycle.activate_pending(),
        Err(ConfigLifecycleError::NoPendingRevision)
    ));

    fs::remove_dir_all(directory).expect("cleanup");
}

fn test_directory() -> PathBuf {
    let sequence = TEST_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "lungyam-config-lifecycle-{}-{sequence}",
        std::process::id()
    ))
}
