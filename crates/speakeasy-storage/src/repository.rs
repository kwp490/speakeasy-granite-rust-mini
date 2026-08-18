use std::collections::VecDeque;
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};

pub const DATABASE_SCHEMA_VERSION: u32 = 1;
const SESSION_RESULT_LIMIT: usize = 50;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResultProvenance {
    Raw,
    Polished,
    FinalizedStream,
    LastValidDraft,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TranscriptResult {
    pub session_id: String,
    pub created_unix_ms: i64,
    pub raw_text: String,
    pub polished_text: Option<String>,
    pub provenance: ResultProvenance,
    pub secure_target: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct HistoryPolicy {
    pub enabled: bool,
    pub retention_days: u16,
    pub plaintext_disclosure_accepted: bool,
}

impl Default for HistoryPolicy {
    fn default() -> Self {
        Self {
            enabled: false,
            retention_days: 30,
            plaintext_disclosure_accepted: false,
        }
    }
}

impl HistoryPolicy {
    fn permits_persistence(&self) -> bool {
        self.enabled
            && self.plaintext_disclosure_accepted
            && (1..=365).contains(&self.retention_days)
    }
}

#[derive(Debug)]
pub enum RepositoryError {
    Io(io::Error),
    Sql(rusqlite::Error),
    TooNew(u32),
    InvalidPolicy,
    InvalidResult,
}

impl From<io::Error> for RepositoryError {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}

impl From<rusqlite::Error> for RepositoryError {
    fn from(value: rusqlite::Error) -> Self {
        Self::Sql(value)
    }
}

#[derive(Debug, Default)]
pub struct SessionResultList {
    results: VecDeque<TranscriptResult>,
}

impl SessionResultList {
    /// Adds one validated result to the bounded in-memory list.
    ///
    /// # Errors
    ///
    /// Returns an invalid-result error for empty, oversized, or invalid input.
    pub fn push(&mut self, result: TranscriptResult) -> Result<(), RepositoryError> {
        validate_result(&result)?;
        if self.results.len() == SESSION_RESULT_LIMIT {
            self.results.pop_back();
        }
        self.results.push_front(result);
        Ok(())
    }

    #[must_use]
    pub fn list(&self) -> Vec<TranscriptResult> {
        self.results.iter().cloned().collect()
    }

    pub fn clear(&mut self) {
        self.results.clear();
    }
}

pub struct HistoryRepository {
    connection: Connection,
    path: PathBuf,
    policy: HistoryPolicy,
}

impl HistoryRepository {
    /// Opens, configures, migrates, and validates the app-owned history database.
    ///
    /// # Errors
    ///
    /// Returns an I/O, `SQLite`, policy, migration, or too-new-schema error.
    pub fn open(path: impl Into<PathBuf>, policy: HistoryPolicy) -> Result<Self, RepositoryError> {
        validate_policy(&policy)?;
        let path = path.into();
        let parent = path.parent().ok_or(RepositoryError::InvalidPolicy)?;
        fs::create_dir_all(parent)?;
        let existed = path.exists();
        let mut connection = Connection::open(&path)?;
        configure_connection(&connection)?;
        let version = database_version(&connection)?;
        if version > DATABASE_SCHEMA_VERSION {
            return Err(RepositoryError::TooNew(version));
        }
        if existed && version < DATABASE_SCHEMA_VERSION {
            let backup_path = path.with_extension("sqlite3.pre-migration.bak");
            connection.backup("main", &backup_path, None)?;
        }
        migrate(&mut connection, version)?;
        Ok(Self {
            connection,
            path,
            policy,
        })
    }

    pub fn policy(&self) -> &HistoryPolicy {
        &self.policy
    }

    /// Replaces the in-memory persistence policy.
    ///
    /// # Errors
    ///
    /// Returns an invalid-policy error for an unsupported retention value.
    pub fn set_policy(&mut self, policy: HistoryPolicy) -> Result<(), RepositoryError> {
        validate_policy(&policy)?;
        self.policy = policy;
        Ok(())
    }

    /// Returns true only when a non-sensitive result was written.
    ///
    /// # Errors
    ///
    /// Returns a validation or `SQLite` write error.
    pub fn record(&mut self, result: &TranscriptResult) -> Result<bool, RepositoryError> {
        validate_result(result)?;
        if !self.policy.permits_persistence() || result.secure_target {
            return Ok(false);
        }
        self.connection.execute(
            "INSERT OR REPLACE INTO transcript_history
             (session_id, created_unix_ms, raw_text, polished_text, provenance)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                result.session_id,
                result.created_unix_ms,
                result.raw_text,
                result.polished_text,
                provenance_name(&result.provenance)
            ],
        )?;
        Ok(true)
    }

    /// Returns at most 500 persisted results in newest-first order.
    ///
    /// # Errors
    ///
    /// Returns a `SQLite` read or row-decoding error.
    pub fn list(&self, limit: usize) -> Result<Vec<TranscriptResult>, RepositoryError> {
        let bounded = limit.clamp(1, 500);
        let mut statement = self.connection.prepare(
            "SELECT session_id, created_unix_ms, raw_text, polished_text, provenance
             FROM transcript_history ORDER BY created_unix_ms DESC LIMIT ?1",
        )?;
        let rows = statement.query_map([i64::try_from(bounded).unwrap_or(500)], |row| {
            let provenance: String = row.get(4)?;
            Ok(TranscriptResult {
                session_id: row.get(0)?,
                created_unix_ms: row.get(1)?,
                raw_text: row.get(2)?,
                polished_text: row.get(3)?,
                provenance: parse_provenance(&provenance),
                secure_target: false,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(RepositoryError::Sql)
    }

    /// Deletes rows older than the configured retention window.
    ///
    /// # Errors
    ///
    /// Returns a policy or `SQLite` deletion error.
    pub fn apply_retention(&mut self, now_unix_ms: i64) -> Result<usize, RepositoryError> {
        validate_policy(&self.policy)?;
        if !self.policy.permits_persistence() {
            return Ok(0);
        }
        let cutoff = now_unix_ms.saturating_sub(
            i64::from(self.policy.retention_days).saturating_mul(24 * 60 * 60 * 1_000),
        );
        self.connection
            .execute(
                "DELETE FROM transcript_history WHERE created_unix_ms < ?1",
                [cutoff],
            )
            .map_err(RepositoryError::Sql)
    }

    /// Exports persisted rows to one new absolute-path JSON file.
    ///
    /// # Errors
    ///
    /// Returns an error when disclosure is absent, the path is invalid, the
    /// destination exists, serialization fails, or an I/O/SQLite error occurs.
    pub fn export_json(
        &self,
        destination: &Path,
        disclosure_accepted: bool,
    ) -> Result<usize, RepositoryError> {
        if !disclosure_accepted || !destination.is_absolute() {
            return Err(RepositoryError::InvalidPolicy);
        }
        let rows = self.list(500)?;
        let bytes = serde_json::to_vec_pretty(&rows).map_err(|_| RepositoryError::InvalidResult)?;
        let mut file = File::create_new(destination)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        Ok(rows.len())
    }

    /// Securely removes all persisted history and compacts the database.
    ///
    /// # Errors
    ///
    /// Returns a `SQLite` or sidecar-removal error.
    pub fn delete_all(self) -> Result<(), RepositoryError> {
        self.connection.execute_batch(
            "PRAGMA secure_delete=ON;
             BEGIN IMMEDIATE;
             DELETE FROM transcript_history;
             COMMIT;
             PRAGMA wal_checkpoint(TRUNCATE);
             VACUUM;",
        )?;
        drop(self.connection);
        for suffix in ["-wal", "-shm"] {
            let sidecar = PathBuf::from(format!("{}{}", self.path.display(), suffix));
            if sidecar.exists() {
                fs::remove_file(sidecar)?;
            }
        }
        Ok(())
    }
}

fn configure_connection(connection: &Connection) -> Result<(), rusqlite::Error> {
    connection.pragma_update(None, "foreign_keys", true)?;
    connection.pragma_update(None, "journal_mode", "WAL")?;
    connection.busy_timeout(Duration::from_secs(2))
}

fn database_version(connection: &Connection) -> Result<u32, RepositoryError> {
    let has_meta: Option<i64> = connection
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type='table' AND name='schema_meta'",
            [],
            |row| row.get(0),
        )
        .optional()?;
    if has_meta.is_none() {
        return Ok(0);
    }
    let version: i64 =
        connection.query_row("SELECT version FROM schema_meta LIMIT 1", [], |row| {
            row.get(0)
        })?;
    u32::try_from(version).map_err(|_| RepositoryError::InvalidResult)
}

fn migrate(connection: &mut Connection, from: u32) -> Result<(), RepositoryError> {
    if from == 0 {
        let transaction = connection.transaction()?;
        transaction.execute_batch(
            "CREATE TABLE schema_meta (
                version INTEGER NOT NULL CHECK(version >= 1)
             );
             INSERT INTO schema_meta(version) VALUES (1);
             CREATE TABLE transcript_history (
                session_id TEXT PRIMARY KEY NOT NULL,
                created_unix_ms INTEGER NOT NULL,
                raw_text TEXT NOT NULL,
                polished_text TEXT,
                provenance TEXT NOT NULL
             );
             CREATE INDEX transcript_history_created
                ON transcript_history(created_unix_ms);",
        )?;
        transaction.commit()?;
    }
    Ok(())
}

fn validate_policy(policy: &HistoryPolicy) -> Result<(), RepositoryError> {
    if !(1..=365).contains(&policy.retention_days) {
        return Err(RepositoryError::InvalidPolicy);
    }
    Ok(())
}

fn validate_result(result: &TranscriptResult) -> Result<(), RepositoryError> {
    if result.session_id.trim().is_empty()
        || result.session_id.len() > 128
        || result.raw_text.is_empty()
        || result.raw_text.len() > 1_000_000
        || result
            .polished_text
            .as_ref()
            .is_some_and(|text| text.len() > 1_000_000)
    {
        return Err(RepositoryError::InvalidResult);
    }
    Ok(())
}

const fn provenance_name(provenance: &ResultProvenance) -> &'static str {
    match provenance {
        ResultProvenance::Raw => "raw",
        ResultProvenance::Polished => "polished",
        ResultProvenance::FinalizedStream => "finalized_stream",
        ResultProvenance::LastValidDraft => "last_valid_draft",
    }
}

fn parse_provenance(value: &str) -> ResultProvenance {
    match value {
        "polished" => ResultProvenance::Polished,
        "finalized_stream" => ResultProvenance::FinalizedStream,
        "last_valid_draft" => ResultProvenance::LastValidDraft,
        _ => ResultProvenance::Raw,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn result(id: &str, time: i64, secure: bool) -> TranscriptResult {
        TranscriptResult {
            session_id: id.to_owned(),
            created_unix_ms: time,
            raw_text: "fixture raw".to_owned(),
            polished_text: Some("fixture polished".to_owned()),
            provenance: ResultProvenance::Polished,
            secure_target: secure,
        }
    }

    #[test]
    fn session_results_are_bounded_and_never_require_sqlite() {
        let mut list = SessionResultList::default();
        for index in 0..60 {
            list.push(result(&format!("s-{index}"), index, false))
                .unwrap();
        }
        assert_eq!(list.list().len(), SESSION_RESULT_LIMIT);
        assert_eq!(list.list()[0].session_id, "s-59");
        list.clear();
        assert!(list.list().is_empty());
    }

    #[test]
    fn history_is_off_by_default_and_secure_results_are_always_excluded() {
        let root = tempdir().unwrap();
        let path = root.path().join("history.sqlite3");
        let mut repository = HistoryRepository::open(&path, HistoryPolicy::default()).unwrap();
        assert!(!repository.record(&result("off", 1, false)).unwrap());
        repository
            .set_policy(HistoryPolicy {
                enabled: true,
                retention_days: 30,
                plaintext_disclosure_accepted: true,
            })
            .unwrap();
        assert!(!repository.record(&result("secure", 2, true)).unwrap());
        assert!(repository.record(&result("normal", 3, false)).unwrap());
        let rows = repository.list(10).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].session_id, "normal");
        assert_eq!(rows[0].polished_text.as_deref(), Some("fixture polished"));
    }

    #[test]
    fn retention_export_delete_and_too_new_recovery_are_safe() {
        let root = tempdir().unwrap();
        let path = root.path().join("history.sqlite3");
        let policy = HistoryPolicy {
            enabled: true,
            retention_days: 1,
            plaintext_disclosure_accepted: true,
        };
        let mut repository = HistoryRepository::open(&path, policy).unwrap();
        repository.record(&result("old", 1, false)).unwrap();
        repository
            .record(&result("new", 2 * 24 * 60 * 60 * 1_000, false))
            .unwrap();
        assert_eq!(
            repository
                .apply_retention(2 * 24 * 60 * 60 * 1_000)
                .unwrap(),
            1
        );
        let export = root.path().join("history.json");
        assert_eq!(repository.export_json(&export, true).unwrap(), 1);
        let exported = fs::read_to_string(export).unwrap();
        assert!(exported.contains("fixture raw"));
        assert!(!exported.contains("\"session_id\": \"secure\""));
        repository.delete_all().unwrap();

        let connection = Connection::open(&path).unwrap();
        connection
            .execute("UPDATE schema_meta SET version=99", [])
            .unwrap();
        drop(connection);
        assert!(matches!(
            HistoryRepository::open(&path, HistoryPolicy::default()),
            Err(RepositoryError::TooNew(99))
        ));
    }
}
