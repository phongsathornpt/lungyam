use std::{path::{Path, PathBuf}, sync::Mutex};

use thiserror::Error;

use crate::{
    config::Config,
    config_diff::ConfigDiff,
    revision::{FileRevisionStore, RevisionError, RevisionMetadata},
    revision_state::{FileRevisionStateStore, RevisionState, RevisionStateError},
    store::{ConfigStore, ConfigStoreError, FileConfigStore},
};

static LIFECYCLE_LOCK: Mutex<()> = Mutex::new(());

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActivationResult {
    pub revision: u64,
    pub diff: ConfigDiff,
    pub restart_required: bool,
}

#[derive(Clone, Debug)]
pub struct FileConfigLifecycle {
    active_store: FileConfigStore,
    revision_store: FileRevisionStore,
    state_store: FileRevisionStateStore,
}

impl FileConfigLifecycle {
    #[must_use]
    pub fn new(config_path: impl Into<PathBuf>) -> Self {
        let config_path = config_path.into();
        let active_store = FileConfigStore::new(&config_path);
        let revision_store = FileRevisionStore::beside_config(&config_path);
        let state_store = FileRevisionStateStore::new(revision_store.root());
        Self { active_store, revision_store, state_store }
    }

    #[must_use]
    pub fn config_path(&self) -> &Path { self.active_store.path() }

    #[must_use]
    pub fn revision_store(&self) -> &FileRevisionStore { &self.revision_store }

    #[must_use]
    pub fn state_store(&self) -> &FileRevisionStateStore { &self.state_store }

    pub fn stage(&self, candidate: &Config, actor: Option<String>, reason: Option<String>) -> Result<RevisionMetadata, ConfigLifecycleError> {
        let _guard = LIFECYCLE_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        self.stage_inner(candidate, actor, reason)
    }

    pub fn activate_pending(&self) -> Result<ActivationResult, ConfigLifecycleError> {
        let _guard = LIFECYCLE_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let revision = self.state_store.load()?.pending_revision.ok_or(ConfigLifecycleError::NoPendingRevision)?;
        self.activate_revision_inner(revision)
    }

    pub fn rollback_to(&self, revision: u64, actor: Option<String>, reason: Option<String>) -> Result<ActivationResult, ConfigLifecycleError> {
        let _guard = LIFECYCLE_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let historical = self.revision_store.load(revision)?;
        let reason = reason.or_else(|| Some(format!("rollback to #{revision:06}")));
        let staged = self.stage_inner(&historical.config, actor, reason)?;
        self.activate_revision_inner(staged.revision)
    }

    fn stage_inner(&self, candidate: &Config, actor: Option<String>, reason: Option<String>) -> Result<RevisionMetadata, ConfigLifecycleError> {
        candidate.validate().map_err(|error| ConfigLifecycleError::Config(ConfigStoreError::Config(error)))?;
        let metadata = self.revision_store.create(candidate, actor, reason)?;
        let mut state = self.state_store.load()?;
        state.pending_revision = Some(metadata.revision);
        self.state_store.save(&state)?;
        Ok(metadata)
    }

    fn activate_revision_inner(&self, revision: u64) -> Result<ActivationResult, ConfigLifecycleError> {
        let candidate = self.revision_store.load(revision)?;
        candidate.config.validate().map_err(|error| ConfigLifecycleError::Config(ConfigStoreError::Config(error)))?;
        let previous = self.active_store.load()?;
        let diff = ConfigDiff::between(&previous, &candidate.config);
        let restart_required = diff.restart_required();

        self.active_store.save(&candidate.config)?;
        let next_state = RevisionState { active_revision: Some(revision), pending_revision: None };
        if let Err(state_error) = self.state_store.save(&next_state) {
            return match self.active_store.save(&previous) {
                Ok(()) => Err(ConfigLifecycleError::State(state_error)),
                Err(restore_error) => Err(ConfigLifecycleError::Recovery { state: state_error, restore: restore_error }),
            };
        }

        Ok(ActivationResult { revision, diff, restart_required })
    }
}

#[derive(Debug, Error)]
pub enum ConfigLifecycleError {
    #[error("active configuration error: {0}")]
    Config(#[from] ConfigStoreError),
    #[error("revision error: {0}")]
    Revision(#[from] RevisionError),
    #[error("revision state error: {0}")]
    State(#[from] RevisionStateError),
    #[error("there is no pending configuration revision")]
    NoPendingRevision,
    #[error("revision state update failed and restoring the previous active config also failed: state={state}; restore={restore}")]
    Recovery { state: RevisionStateError, restore: ConfigStoreError },
}
