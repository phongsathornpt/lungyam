use std::{
    cmp::Reverse,
    fs::{self, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::{Mutex, atomic::{AtomicU64, Ordering}},
    time::{SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::config::{Config, ConfigError};

static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(1);
static REVISION_LOCK: Mutex<()> = Mutex::new(());

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct RevisionMetadata {
    pub revision: u64,
    pub created_at_unix_seconds: u64,
    pub actor: Option<String>,
    pub reason: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfigRevision {
    pub metadata: RevisionMetadata,
    pub config: Config,
}

#[derive(Clone, Debug)]
pub struct FileRevisionStore {
    root: PathBuf,
}

impl FileRevisionStore {
    #[must_use]
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    #[must_use]
    pub fn beside_config(config_path: &Path) -> Self {
        let parent = config_path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        Self::new(parent.join("revisions"))
    }

    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn create(
        &self,
        config: &Config,
        actor: Option<String>,
        reason: Option<String>,
    ) -> Result<RevisionMetadata, RevisionError> {
        config.validate().map_err(RevisionError::Config)?;
        let _guard = REVISION_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        fs::create_dir_all(&self.root)?;

        let revision = next_revision(&self.root)?;
        let metadata = RevisionMetadata {
            revision,
            created_at_unix_seconds: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
            actor,
            reason,
        };
        persist(&self.root, config, &metadata)?;
        Ok(metadata)
    }

    pub fn list(&self) -> Result<Vec<RevisionMetadata>, RevisionError> {
        if !self.root.exists() {
            return Ok(Vec::new());
        }

        let mut revisions = Vec::new();
        for entry in fs::read_dir(&self.root)? {
            let entry = entry?;
            if !entry.file_type()?.is_dir() {
                continue;
            }
            let Some(revision) = parse_revision(&entry.file_name()) else {
                continue;
            };
            revisions.push(read_metadata(&entry.path(), revision)?);
        }
        revisions.sort_by_key(|metadata| Reverse(metadata.revision));
        Ok(revisions)
    }

    pub fn load(&self, revision: u64) -> Result<ConfigRevision, RevisionError> {
        let directory = self.root.join(format_revision(revision));
        if !directory.is_dir() {
            return Err(RevisionError::NotFound(revision));
        }
        Ok(ConfigRevision {
            metadata: read_metadata(&directory, revision)?,
            config: Config::from_path(directory.join("config.yaml")).map_err(RevisionError::Config)?,
        })
    }
}

#[derive(Debug, Error)]
pub enum RevisionError {
    #[error("configuration is invalid: {0}")]
    Config(#[source] ConfigError),
    #[error("revision YAML error: {0}")]
    Yaml(#[from] serde_yaml::Error),
    #[error("configuration revision {0} was not found")]
    NotFound(u64),
    #[error("revision storage error: {0}")]
    Io(#[from] io::Error),
}

fn persist(root: &Path, config: &Config, metadata: &RevisionMetadata) -> Result<(), RevisionError> {
    let final_path = root.join(format_revision(metadata.revision));
    let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let temp_path = root.join(format!(
        ".{}.tmp-{}-{sequence}",
        format_revision(metadata.revision),
        std::process::id()
    ));
    fs::create_dir(&temp_path)?;

    let result = (|| -> Result<(), RevisionError> {
        write_synced(&temp_path.join("config.yaml"), &serde_yaml::to_string(config)?)?;
        write_synced(&temp_path.join("metadata.yaml"), &serde_yaml::to_string(metadata)?)?;
        fs::rename(&temp_path, &final_path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(&temp_path);
    }
    result
}

fn write_synced(path: &Path, contents: &str) -> io::Result<()> {
    let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
    file.write_all(contents.as_bytes())?;
    file.flush()?;
    file.sync_all()
}

fn next_revision(root: &Path) -> io::Result<u64> {
    let highest = fs::read_dir(root)?
        .filter_map(Result::ok)
        .filter_map(|entry| parse_revision(&entry.file_name()))
        .max()
        .unwrap_or(0);
    highest
        .checked_add(1)
        .ok_or_else(|| io::Error::other("configuration revision sequence exhausted"))
}

fn read_metadata(directory: &Path, expected: u64) -> Result<RevisionMetadata, RevisionError> {
    let metadata: RevisionMetadata =
        serde_yaml::from_str(&fs::read_to_string(directory.join("metadata.yaml"))?)?;
    if metadata.revision != expected {
        return Err(RevisionError::Io(io::Error::new(
            io::ErrorKind::InvalidData,
            "revision metadata does not match directory",
        )));
    }
    Ok(metadata)
}

fn parse_revision(name: &std::ffi::OsStr) -> Option<u64> {
    name.to_str()?.parse().ok()
}

fn format_revision(revision: u64) -> String {
    format!("{revision:06}")
}
