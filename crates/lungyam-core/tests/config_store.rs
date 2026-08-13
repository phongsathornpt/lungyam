use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

use lungyam_core::{
    config::Config,
    store::{ConfigStore, FileConfigStore},
};

static TEST_SEQUENCE: AtomicU64 = AtomicU64::new(1);

const VALID: &str = r#"
server:
  listen: 127.0.0.1:8080
admin:
  enabled: true
  listen: 127.0.0.1:9090
upstreams:
  api:
    endpoints:
      - 127.0.0.1:3000
routes:
  - name: api
    path: /api
    methods: [GET]
    upstream: api
"#;

#[test]
fn public_file_store_round_trips_validated_config() {
    let directory = test_directory();
    fs::create_dir_all(&directory).expect("create test directory");
    let path = directory.join("lungyam.yaml");
    let store = FileConfigStore::new(&path);
    let config = Config::from_yaml(VALID).expect("valid fixture");

    store.save(&config).expect("persist config");
    let loaded = store.load().expect("load persisted config");

    assert_eq!(loaded, config);
    fs::remove_dir_all(directory).expect("remove test directory");
}

fn test_directory() -> PathBuf {
    let sequence = TEST_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "lungyam-public-config-store-{}-{sequence}",
        std::process::id()
    ))
}
