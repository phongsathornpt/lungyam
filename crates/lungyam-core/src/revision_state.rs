use std::{
    fs::{self, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use serde::{Deserialize, Serialize};
use thiserror::Error;

static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct RevisionState {
    pub active_revision: Option<u64>,
    pub pending_revision: Option<u64>,
}

#[derive(Clone, Debug)]
pub struct FileRevisionStateStore {
    revisions_root: PathBuf,
}

impl FileRevisionStateStore {
    #[must_use]
    pub fn new(revisions_root: impl Into<PathBuf>) -> Self {
        Self {
            revisions_root: revisions_root.into(),
        }
    }

    #[must_use]
    pub fn path(&self) -> PathBuf {
        self.revisions_root.join("state.yaml")
    }

    pub fn load(&self) -> Result<RevisionState, RevisionStateError> {
        let path = self.path();
        if !path.exists() {
            return Ok(RevisionState::default());
        }
        Ok(serde_yaml::from_str(&fs::read_to_string(path)?)?)
    }

    pub fn save(&self, state: &RevisionState) -> Result<(), RevisionStateError> {
        self.validate_reference(state.active_revision)?;
        self.validate_reference(state.pending_revision)?;
        fs::create_dir_all(&self.revisions_root)?;
        atomic_replace(&self.path(), &serde_yaml::to_string(state)?)?;
        Ok(())
    }

    fn validate_reference(&self, revision: Option<u64>) -> Result<(), RevisionStateError> {
        let Some(revision) = revision else {
            return Ok(());
        };
        if self
            .revisions_root
            .join(format!("{revision:06}"))
            .is_dir()
        {
            Ok(())
        } else {
            Err(RevisionStateError::UnknownRevision(revision))
        }
    }
}

#[derive(Debug, Error)]
pub enum RevisionStateError {
    #[error("configuration revision {0} does not exist")]
    UnknownRevision(u64),
    #[error("revision state YAML error: {0}")]
    Yaml(#[from] serde_yaml::Error),
    #[error("revision state storage error: {0}")]
    Io(#[from] io::Error),
}

fn atomic_replace(path: &Path, contents: &str) -> io::Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let temp_path = parent.join(format!(
        ".state.yaml.tmp-{}-{sequence}",
        std::process::id()
    ));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp_path)?;
    file.write_all(contents.as_bytes())?;
    file.flush()?;
    file.sync_all()?;
    drop(file);

    let result = fs::rename(&temp_path, path);
    if result.is_err() {
        let _ = fs::remove_file(temp_path);
    }
    result
}
