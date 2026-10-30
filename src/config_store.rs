//! Runtime configuration store.
//!
//! Holds the active configuration together with an optional, persisted
//! draft. A draft is only ever promoted to the active configuration by
//! [`ConfigStore::apply`], which writes a backup of the current file before
//! replacing it.

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Once, PoisonError, RwLock, RwLockReadGuard, RwLockWriteGuard};

use camino::{Utf8Path, Utf8PathBuf};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::config::diff::{self, ApplyPlan, ConfigChange};
use crate::config::validate::{self, ValidationError};
use crate::config::{self, AppConfig};

/// Permissions for every file the store creates. Configuration may hold
/// secrets, so it is readable by the owner only.
const PRIVATE_FILE_MODE: u32 = 0o600;

const DEFAULT_CONFIG_FILE_NAME: &str = "config.yaml";
const DEFAULT_EXTENSION: &str = "yaml";

static RENAME_FALLBACK_LOGGED: Once = Once::new();

#[derive(Error, Debug)]
pub enum ConfigStoreError {
    #[error("Stale revision {expected}, current revision is {current}")]
    StaleRevision { expected: u64, current: u64 },

    #[error("No draft configuration exists")]
    NoDraft,

    #[error("Configuration has {} validation error(s)", .0.len())]
    Invalid(Vec<ValidationError>),

    #[error(transparent)]
    Io(#[from] io::Error),

    #[error(transparent)]
    Yaml(#[from] serde_yml::Error),

    #[error(transparent)]
    Config(#[from] ::config::ConfigError),
}

pub type ConfigStoreResult<T> = Result<T, ConfigStoreError>;

/// Files the store reads and writes, all relative to the directory of the
/// active configuration file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfigPaths {
    pub config: Utf8PathBuf,
    pub draft: Utf8PathBuf,
    pub backup: Utf8PathBuf,
    pub state: Utf8PathBuf,
    pub cert: Utf8PathBuf,
}

impl ConfigPaths {
    /// Resolve all paths for `config_file`. Relative `state_file` and
    /// `cert_file` settings resolve against the configuration's directory;
    /// for the default `config.yaml` in the working directory this is the
    /// working directory, exactly as before.
    #[must_use]
    pub fn resolve(config_file: &Utf8Path, config: &AppConfig) -> Self {
        let dir = config_file.parent().unwrap_or_else(|| Utf8Path::new(""));
        let file_name = config_file.file_name().unwrap_or(DEFAULT_CONFIG_FILE_NAME);
        let stem = config_file.file_stem().unwrap_or("config");
        let extension = config_file.extension().unwrap_or(DEFAULT_EXTENSION);

        Self {
            config: config_file.to_path_buf(),
            draft: dir.join(format!("{stem}.draft.{extension}")),
            backup: dir.join(format!("{file_name}.bak")),
            state: dir.join(&config.bifrost.state_file),
            cert: dir.join(&config.bifrost.cert_file),
        }
    }
}

/// A configuration that has been saved but not yet applied.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Draft {
    pub config: AppConfig,
    pub revision: u64,
    pub updated_at: DateTime<Utc>,
}

/// Result of promoting the draft to the active configuration.
#[derive(Clone, Debug)]
pub struct ApplyOutcome {
    pub revision: u64,
    pub previous: AppConfig,
    pub applied: AppConfig,
    pub plan: ApplyPlan,
    /// Applied changes, with secrets redacted.
    pub changes: Vec<ConfigChange>,
}

pub struct ConfigStore {
    active: Arc<RwLock<AppConfig>>,
    draft: Arc<RwLock<Option<Draft>>>,
    paths: ConfigPaths,
    revision: AtomicU64,
}

fn read<T>(lock: &RwLock<T>) -> RwLockReadGuard<'_, T> {
    lock.read().unwrap_or_else(PoisonError::into_inner)
}

fn write<T>(lock: &RwLock<T>) -> RwLockWriteGuard<'_, T> {
    lock.write().unwrap_or_else(PoisonError::into_inner)
}

impl ConfigStore {
    /// Load the active configuration from `config_file`, using the same
    /// parser and defaults as always, and pick up a persisted draft if one
    /// exists next to it.
    pub fn load(config_file: &Utf8Path) -> ConfigStoreResult<Self> {
        let active = config::parse(config_file)?;
        Ok(Self::new(config_file, active))
    }

    /// Create a store for an already parsed configuration. A draft file that
    /// cannot be read is reported and ignored, so it can never prevent
    /// startup.
    #[must_use]
    pub fn new(config_file: &Utf8Path, active: AppConfig) -> Self {
        let paths = ConfigPaths::resolve(config_file, &active);

        let draft = match Self::read_draft(&paths.draft) {
            Ok(draft) => draft,
            Err(err) => {
                log::warn!(
                    "Ignoring unreadable draft configuration {}: {err}",
                    paths.draft
                );
                None
            }
        };

        let revision = draft.as_ref().map_or(0, |draft| draft.revision);

        Self {
            active: Arc::new(RwLock::new(active)),
            draft: Arc::new(RwLock::new(draft)),
            paths,
            revision: AtomicU64::new(revision),
        }
    }

    fn read_draft(path: &Utf8Path) -> ConfigStoreResult<Option<Draft>> {
        match fs::read_to_string(path) {
            Ok(text) => Ok(Some(serde_yml::from_str(&text)?)),
            Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(err) => Err(err.into()),
        }
    }

    #[must_use]
    pub const fn paths(&self) -> &ConfigPaths {
        &self.paths
    }

    /// Snapshot of the active configuration.
    #[must_use]
    pub fn active(&self) -> AppConfig {
        read(&self.active).clone()
    }

    /// Snapshot of the current draft, if any.
    #[must_use]
    pub fn draft(&self) -> Option<Draft> {
        read(&self.draft).clone()
    }

    /// Revision every write must name; bumped by each draft write, draft
    /// discard and apply.
    #[must_use]
    pub fn revision(&self) -> u64 {
        self.revision.load(Ordering::SeqCst)
    }

    fn check_revision(&self, expected: u64) -> ConfigStoreResult<()> {
        let current = self.revision();
        if expected == current {
            Ok(())
        } else {
            Err(ConfigStoreError::StaleRevision { expected, current })
        }
    }

    fn next_revision(&self) -> u64 {
        self.revision.fetch_add(1, Ordering::SeqCst) + 1
    }

    /// Replace the draft with `config` and persist it.
    // the revision is bumped while the draft lock is held, so a concurrent
    // writer cannot pass the revision check in between
    #[allow(clippy::significant_drop_tightening)]
    pub fn set_draft(&self, expected_revision: u64, config: AppConfig) -> ConfigStoreResult<Draft> {
        let mut draft = write(&self.draft);
        self.check_revision(expected_revision)?;

        let next = Draft {
            config,
            revision: self.revision() + 1,
            updated_at: Utc::now(),
        };
        write_atomic(&self.paths.draft, serde_yml::to_string(&next)?.as_bytes())?;

        self.next_revision();
        *draft = Some(next.clone());

        Ok(next)
    }

    /// Throw away the draft, returning the new revision.
    // the revision is bumped while the draft lock is held, so a concurrent
    // writer cannot pass the revision check in between
    #[allow(clippy::significant_drop_tightening)]
    pub fn discard_draft(&self, expected_revision: u64) -> ConfigStoreResult<u64> {
        let mut draft = write(&self.draft);
        self.check_revision(expected_revision)?;

        if draft.is_none() {
            return Err(ConfigStoreError::NoDraft);
        }

        remove_if_exists(&self.paths.draft)?;
        *draft = None;

        Ok(self.next_revision())
    }

    /// Promote the draft to the active configuration.
    ///
    /// Refuses a draft with hard validation errors the active configuration
    /// does not already have. Writes `<config>.bak` from the file currently
    /// on disk, then the new configuration, then swaps the in-memory copy
    /// and clears the draft.
    // the revision is bumped while the draft lock is held, so a concurrent
    // writer cannot pass the revision check in between
    #[allow(clippy::significant_drop_tightening)]
    pub fn apply(&self, expected_revision: u64) -> ConfigStoreResult<ApplyOutcome> {
        let mut draft = write(&self.draft);
        self.check_revision(expected_revision)?;

        let mut applied = draft
            .as_ref()
            .map(|draft| draft.config.clone())
            .ok_or(ConfigStoreError::NoDraft)?;

        // tokens given in the url query move to the token field on the
        // first apply; get_url() builds the same url from either
        if applied.migrate_url_tokens() {
            log::info!("Moved z2m tokens from url queries to the token field");
        }

        let errors = validate::new_errors(&read(&self.active), &applied);
        if !errors.is_empty() {
            return Err(ConfigStoreError::Invalid(errors));
        }

        let yaml = serde_yml::to_string(&applied)?;
        self.backup()?;
        write_atomic(&self.paths.config, yaml.as_bytes())?;

        let previous = std::mem::replace(&mut *write(&self.active), applied.clone());
        *draft = None;

        // The new configuration is already active; a draft file that cannot
        // be removed would only resurface as a draft after restart.
        if let Err(err) = remove_if_exists(&self.paths.draft) {
            log::warn!("Failed to remove applied draft {}: {err}", self.paths.draft);
        }

        Ok(ApplyOutcome {
            revision: self.next_revision(),
            plan: diff::plan(&migrated(&previous), &applied),
            changes: diff::diff(&migrated(&previous), &applied),
            previous,
            applied,
        })
    }

    fn backup(&self) -> io::Result<()> {
        match fs::read(&self.paths.config) {
            Ok(current) => write_atomic(&self.paths.backup, &current),
            Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(err) => Err(err),
        }
    }
}

/// `config` with url tokens migrated, so the migration alone never shows
/// up as a change.
fn migrated(config: &AppConfig) -> AppConfig {
    let mut config = config.clone();
    config.migrate_url_tokens();
    config
}

fn remove_if_exists(path: &Utf8Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Err(err) if err.kind() != io::ErrorKind::NotFound => Err(err),
        _ => Ok(()),
    }
}

fn write_private(path: &Utf8Path, data: &[u8]) -> io::Result<()> {
    let mut fd = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(PRIVATE_FILE_MODE)
        .open(path)?;
    fd.write_all(data)?;
    fd.sync_all()
}

/// Atomically replace `path` with `data`.
///
/// The data goes to a private temp file in the same directory, which is then
/// renamed over the target. When the target is a single-file bind mount (as
/// in the shipped `docker-compose.yaml`), the rename fails with `EBUSY`; in
/// that case the file is truncated and written in place instead.
pub fn write_atomic(path: &Utf8Path, data: &[u8]) -> io::Result<()> {
    write_atomic_with(path, data, |from, to| fs::rename(from, to))
}

fn write_atomic_with(
    path: &Utf8Path,
    data: &[u8],
    rename: impl Fn(&Path, &Path) -> io::Result<()>,
) -> io::Result<()> {
    let dir = path.parent().unwrap_or_else(|| Utf8Path::new(""));
    let file_name = path.file_name().unwrap_or(DEFAULT_CONFIG_FILE_NAME);
    let tmp = dir.join(format!(".{file_name}.tmp"));

    // a leftover temp file could carry wider permissions; start fresh
    remove_if_exists(&tmp)?;
    write_private(&tmp, data)?;

    if let Err(err) = rename(tmp.as_std_path(), path.as_std_path()) {
        let _ = fs::remove_file(&tmp);
        RENAME_FALLBACK_LOGGED.call_once(|| {
            log::info!(
                "Cannot replace {path} by rename ({err}); writing in place instead. \
                 This is expected when the file is a bind mount."
            );
        });
        write_private(path, data)?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::io;
    use std::os::unix::fs::PermissionsExt;

    use camino::Utf8Path;

    use crate::config::diff::ApplyAction;
    use crate::config_store::{
        ConfigPaths, ConfigStore, ConfigStoreError, write_atomic, write_atomic_with,
    };
    use crate::testutil::{MINIMAL_CONFIG, TempDir};

    fn store_in(dir: &TempDir) -> ConfigStore {
        let path = dir.write("config.yaml", MINIMAL_CONFIG);
        ConfigStore::load(&path).unwrap()
    }

    fn mode(path: &Utf8Path) -> u32 {
        fs::metadata(path).unwrap().permissions().mode() & 0o777
    }

    #[test]
    fn paths_default_to_working_directory() {
        let dir = TempDir::new();
        let store = store_in(&dir);
        let config = store.active();

        let paths = ConfigPaths::resolve(Utf8Path::new("config.yaml"), &config);

        assert_eq!(paths.config, "config.yaml");
        assert_eq!(paths.draft, "config.draft.yaml");
        assert_eq!(paths.backup, "config.yaml.bak");
        assert_eq!(paths.state, "state.yaml");
        assert_eq!(paths.cert, "cert.pem");
    }

    #[test]
    fn paths_resolve_relative_to_config_directory() {
        let dir = TempDir::new();
        let store = store_in(&dir);
        let mut config = store.active();
        config.bifrost.cert_file = "certs/cert.pem".into();
        config.bifrost.state_file = "/var/lib/bifrost/state.yaml".into();

        let paths = ConfigPaths::resolve(Utf8Path::new("/etc/bifrost/bifrost.yml"), &config);

        assert_eq!(paths.draft, "/etc/bifrost/bifrost.draft.yml");
        assert_eq!(paths.backup, "/etc/bifrost/bifrost.yml.bak");
        assert_eq!(paths.cert, "/etc/bifrost/certs/cert.pem");
        assert_eq!(paths.state, "/var/lib/bifrost/state.yaml");
    }

    #[test]
    fn draft_is_persisted_and_survives_restart() {
        let dir = TempDir::new();
        let store = store_in(&dir);
        let mut config = store.active();
        config.bridge.name = "Draft name".into();

        let draft = store.set_draft(0, config).unwrap();
        assert_eq!(draft.revision, 1);
        assert_eq!(mode(&store.paths().draft), 0o600);

        let restarted = ConfigStore::load(&store.paths().config).unwrap();
        let reloaded = restarted.draft().unwrap();
        assert_eq!(reloaded.config.bridge.name, "Draft name");
        assert_eq!(reloaded.revision, 1);
        assert_eq!(restarted.revision(), 1);

        // a draft is never applied implicitly
        assert_eq!(restarted.active().bridge.name, "Bifrost");
    }

    #[test]
    fn stale_revision_is_rejected() {
        let dir = TempDir::new();
        let store = store_in(&dir);

        store.set_draft(0, store.active()).unwrap();
        let err = store.set_draft(0, store.active()).unwrap_err();

        assert!(matches!(
            err,
            ConfigStoreError::StaleRevision {
                expected: 0,
                current: 1
            }
        ));
        assert!(matches!(
            store.apply(0).unwrap_err(),
            ConfigStoreError::StaleRevision { .. }
        ));
    }

    #[test]
    fn apply_without_draft_fails() {
        let dir = TempDir::new();
        let store = store_in(&dir);

        assert!(matches!(
            store.apply(0).unwrap_err(),
            ConfigStoreError::NoDraft
        ));
    }

    #[test]
    fn apply_writes_backup_and_updates_active() {
        let dir = TempDir::new();
        let store = store_in(&dir);
        let mut config = store.active();
        config.bridge.name = "Applied".into();
        store.set_draft(0, config).unwrap();

        let outcome = store.apply(1).unwrap();

        assert_eq!(outcome.revision, 2);
        assert_eq!(outcome.previous.bridge.name, "Bifrost");
        assert_eq!(outcome.applied.bridge.name, "Applied");
        assert_eq!(outcome.changes.len(), 1);
        assert_eq!(
            outcome.plan.hot,
            [ApplyAction::Update {
                path: "bridge.name".into()
            }]
        );
        assert_eq!(store.active().bridge.name, "Applied");
        assert!(store.draft().is_none());
        assert!(!store.paths().draft.exists());

        let backup = fs::read_to_string(&store.paths().backup).unwrap();
        assert_eq!(backup, MINIMAL_CONFIG);
        assert_eq!(mode(&store.paths().backup), 0o600);

        let reloaded = ConfigStore::load(&store.paths().config).unwrap();
        assert_eq!(reloaded.active().bridge.name, "Applied");
    }

    #[test]
    fn apply_refuses_new_validation_errors() {
        let dir = TempDir::new();
        let store = store_in(&dir);
        let mut config = store.active();
        config.bridge.https_port = config.bridge.http_port;
        store.set_draft(0, config).unwrap();

        let err = store.apply(1).unwrap_err();

        let ConfigStoreError::Invalid(errors) = err else {
            panic!("expected validation failure, got {err:?}");
        };
        assert_eq!(errors[0].path, "bridge.https_port");
        assert!(store.draft().is_some());
        assert_eq!(store.revision(), 1);
        assert!(!store.paths().backup.exists());
    }

    #[test]
    fn discard_removes_draft() {
        let dir = TempDir::new();
        let store = store_in(&dir);
        store.set_draft(0, store.active()).unwrap();

        assert_eq!(store.discard_draft(1).unwrap(), 2);
        assert!(store.draft().is_none());
        assert!(!store.paths().draft.exists());
        assert!(matches!(
            store.discard_draft(2).unwrap_err(),
            ConfigStoreError::NoDraft
        ));
    }

    #[test]
    fn unreadable_draft_is_ignored() {
        let dir = TempDir::new();
        let path = dir.write("config.yaml", MINIMAL_CONFIG);
        let _ = dir.write("config.draft.yaml", "this is: [not a draft");

        let store = ConfigStore::load(&path).unwrap();

        assert!(store.draft().is_none());
        assert_eq!(store.revision(), 0);
    }

    #[test]
    fn atomic_write_creates_private_file() {
        let dir = TempDir::new();
        let path = dir.path().join("out.yaml");

        write_atomic(&path, b"first").unwrap();
        write_atomic(&path, b"second").unwrap();

        assert_eq!(fs::read_to_string(&path).unwrap(), "second");
        assert_eq!(mode(&path), 0o600);
        assert!(!dir.path().join(".out.yaml.tmp").exists());
    }

    #[test]
    fn atomic_write_falls_back_when_rename_is_busy() {
        let dir = TempDir::new();
        let path = dir.write("config.yaml", "old");

        write_atomic_with(&path, b"new", |_, _| {
            Err(io::Error::from(io::ErrorKind::ResourceBusy))
        })
        .unwrap();

        assert_eq!(fs::read_to_string(&path).unwrap(), "new");
        assert!(!dir.path().join(".config.yaml.tmp").exists());
    }
}
