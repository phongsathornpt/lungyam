use std::{
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use thiserror::Error;

use crate::config::{Config, ConfigError};

static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(1);

/// Persistence boundary for validated Lungyam configuration.
pub trait ConfigStore: Send + Sync {
    fn load(&self) -> Result<Config, ConfigStoreError>;
    fn save(&self, config: &Config) -> Result<(), ConfigStoreError>;
}

/// File-backed configuration store that replaces the target atomically.
#[derive(Clone, Debug)]
pub struct FileConfigStore {
    path: PathBuf,
}

impl FileConfigStore {
    #[must_use]
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl ConfigStore for FileConfigStore {
    fn load(&self) -> Result<Config, ConfigStoreError> {
        Config::from_path(&self.path).map_err(ConfigStoreError::Config)
    }

    fn save(&self, config: &Config) -> Result<(), ConfigStoreError> {
        config.validate().map_err(ConfigStoreError::Config)?;
        let yaml = serde_yaml::to_string(config)?;
        atomic_replace(&self.path, yaml.as_bytes())?;
        Ok(())
    }
}

#[derive(Debug, Error)]
pub enum ConfigStoreError {
    #[error("configuration is invalid: {0}")]
    Config(#[source] ConfigError),
    #[error("failed to serialize configuration: {0}")]
    Serialize(#[from] serde_yaml::Error),
    #[error("failed to persist configuration: {0}")]
    Io(#[from] io::Error),
}

fn atomic_replace(path: &Path, contents: &[u8]) -> io::Result<()> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let file_name = path.file_name().ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidInput, "config path has no file name")
    })?;
    let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let temp_path = parent.join(format!(
        ".{}.tmp-{}-{sequence}",
        file_name.to_string_lossy(),
        std::process::id()
    ));

    let result = write_and_replace(path, parent, &temp_path, contents);
    if result.is_err() {
        let _ = fs::remove_file(&temp_path);
    }
    result
}

fn write_and_replace(
    path: &Path,
    parent: &Path,
    temp_path: &Path,
    contents: &[u8],
) -> io::Result<()> {
    let mut temp = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(temp_path)?;
    temp.write_all(contents)?;
    temp.flush()?;
    temp.sync_all()?;
    drop(temp);

    if let Ok(metadata) = fs::metadata(path) {
        fs::set_permissions(temp_path, metadata.permissions())?;
    }

    fs::rename(temp_path, path)?;
    sync_parent_directory(parent)
}

#[cfg(unix)]
fn sync_parent_directory(parent: &Path) -> io::Result<()> {
    File::open(parent)?.sync_all()
}

#[cfg(not(unix))]
fn sync_parent_directory(_parent: &Path) -> io::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::{Path, PathBuf},
        sync::atomic::{AtomicU64, Ordering},
    };

    use crate::config::Config;

    use super::{ConfigStore, FileConfigStore};

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
    fn round_trips_valid_config() {
        let directory = test_directory();
        fs::create_dir_all(&directory).expect("create test directory");
        let path = directory.join("lungyam.yaml");
        let store = FileConfigStore::new(&path);
        let config = Config::from_yaml(VALID).expect("valid fixture");

        store.save(&config).expect("save valid config");
        let loaded = store.load().expect("load stored config");

        assert_eq!(loaded, config);
        assert_eq!(store.path(), path.as_path());
        assert_no_temp_files(&directory);
        fs::remove_dir_all(directory).expect("remove test directory");
    }

    #[test]
    fn invalid_config_does_not_replace_existing_file() {
        let directory = test_directory();
        fs::create_dir_all(&directory).expect("create test directory");
        let path = directory.join("lungyam.yaml");
        let store = FileConfigStore::new(&path);
        let config = Config::from_yaml(VALID).expect("valid fixture");
        store.save(&config).expect("save initial config");
        let before = fs::read(&path).expect("read initial config");

        let mut invalid = config;
        invalid.routes[0].path = "missing-slash".to_owned();
        assert!(store.save(&invalid).is_err());

        let after = fs::read(&path).expect("read config after rejected save");
        assert_eq!(after, before);
        assert_no_temp_files(&directory);
        fs::remove_dir_all(directory).expect("remove test directory");
    }

    fn assert_no_temp_files(directory: &Path) {
        let leftovers: Vec<_> = fs::read_dir(directory)
            .expect("read test directory")
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().contains(".tmp-"))
            .collect();
        assert!(leftovers.is_empty(), "temporary files were left behind");
    }

    fn test_directory() -> PathBuf {
        let sequence = TEST_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "lungyam-config-store-{}-{sequence}",
            std::process::id()
        ))
    }
}
