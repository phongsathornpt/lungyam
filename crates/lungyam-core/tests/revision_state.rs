use std::{
    collections::BTreeMap,
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

use lungyam_core::{
    config::{AdminConfig, Config, RouteConfig, RoutePolicies, ServerConfig, UpstreamConfig},
    config_diff::ConfigDiff,
    revision::FileRevisionStore,
    revision_state::{FileRevisionStateStore, RevisionState},
};

static TEST_SEQUENCE: AtomicU64 = AtomicU64::new(1);

#[test]
fn route_only_diff_does_not_require_restart() {
    let current = test_config();
    let mut candidate = current.clone();
    candidate.routes[0].priority = 50;
    candidate.routes.push(RouteConfig {
        name: "extra".to_owned(),
        path: "/extra".to_owned(),
        upstream: "api".to_owned(),
        host: None,
        methods: vec!["GET".to_owned()],
        priority: 0,
        policies: RoutePolicies::default(),
    });

    let diff = ConfigDiff::between(&current, &candidate);
    assert_eq!(diff.routes_changed, vec!["api"]);
    assert_eq!(diff.routes_added, vec!["extra"]);
    assert!(!diff.is_empty());
    assert!(!diff.restart_required());
}

#[test]
fn upstream_change_requires_restart() {
    let current = test_config();
    let mut candidate = current.clone();
    candidate
        .upstreams
        .get_mut("api")
        .expect("api upstream")
        .read_timeout_ms = Some(5000);

    let diff = ConfigDiff::between(&current, &candidate);
    assert_eq!(diff.upstreams_changed, vec!["api"]);
    assert!(diff.restart_required());
}

#[test]
fn revision_state_round_trips_and_rejects_unknown_references() {
    let directory = test_directory();
    fs::create_dir_all(&directory).expect("create test directory");
    let revision_store = FileRevisionStore::new(directory.join("revisions"));
    let config = test_config();
    let first = revision_store
        .create(&config, None, Some("active".to_owned()))
        .expect("create active revision");
    let mut changed = config;
    changed.routes[0].priority = 50;
    let second = revision_store
        .create(&changed, None, Some("pending".to_owned()))
        .expect("create pending revision");

    let state_store = FileRevisionStateStore::new(revision_store.root());
    let state = RevisionState {
        active_revision: Some(first.revision),
        pending_revision: Some(second.revision),
    };
    state_store.save(&state).expect("save revision state");
    assert_eq!(state_store.load().expect("load revision state"), state);

    let invalid = RevisionState {
        active_revision: Some(99),
        pending_revision: None,
    };
    assert!(state_store.save(&invalid).is_err());
    assert_eq!(state_store.load().expect("load unchanged state"), state);

    fs::remove_dir_all(directory).expect("cleanup test directory");
}

fn test_directory() -> PathBuf {
    let sequence = TEST_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "lungyam-revision-state-{}-{sequence}",
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
        admin: AdminConfig::default(),
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
