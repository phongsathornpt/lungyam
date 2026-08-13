use std::{
    collections::BTreeMap,
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

use lungyam_core::{
    config::{AdminConfig, Config, RouteConfig, RoutePolicies, ServerConfig, UpstreamConfig},
    revision::FileRevisionStore,
};

static TEST_SEQUENCE: AtomicU64 = AtomicU64::new(1);

#[test]
fn revision_store_round_trips_history_and_rejects_invalid_config() {
    let directory = test_directory();
    fs::create_dir_all(&directory).expect("create test directory");
    let store = FileRevisionStore::new(directory.join("revisions"));
    let config = test_config();

    let first = store
        .create(
            &config,
            Some("admin".to_owned()),
            Some("initial snapshot".to_owned()),
        )
        .expect("create first revision");
    let mut changed = config.clone();
    changed.routes[0].priority = 50;
    let second = store
        .create(&changed, None, Some("raise priority".to_owned()))
        .expect("create second revision");

    assert_eq!(first.revision, 1);
    assert_eq!(first.actor.as_deref(), Some("admin"));
    assert!(first.created_at_unix_seconds > 0);
    assert_eq!(second.revision, 2);

    let listed = store.list().expect("list revisions");
    assert_eq!(listed.iter().map(|item| item.revision).collect::<Vec<_>>(), vec![2, 1]);
    assert_eq!(store.load(2).expect("load revision").config, changed);

    let mut invalid = config;
    invalid.routes[0].path = "missing-slash".to_owned();
    assert!(store.create(&invalid, None, None).is_err());
    assert_eq!(store.list().expect("list after rejected revision").len(), 2);

    fs::remove_dir_all(directory).expect("cleanup test directory");
}

fn test_directory() -> PathBuf {
    let sequence = TEST_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "lungyam-revision-store-{}-{sequence}",
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
