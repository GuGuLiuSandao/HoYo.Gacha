use std::collections::HashSet;
use std::fmt::Debug;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;
use std::{env, fmt};

use async_trait::async_trait;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use sqlx::encode::IsNull;
use sqlx::error::BoxDynError;
use sqlx::query::{Query, QueryAs};
use sqlx::sqlite::{
  Sqlite, SqliteArguments, SqliteConnectOptions, SqlitePool, SqliteQueryResult, SqliteRow,
  SqliteTypeInfo, SqliteValueRef,
};
use sqlx::{Decode, Encode, Executor, FromRow, Row, Type};
use tauri::{Emitter, State as TauriState};
use time::OffsetDateTime;
use tokio::sync::{RwLock, mpsc};
use tracing::info;

use crate::consts;
use crate::database::legacy_migration::{LegacyMigrationError, MigrationMetrics};
use crate::error::{Error, ErrorDetails};
use crate::models::{Account, Business, GachaRecord, Kv, Properties};

mod kvs;
mod legacy_migration;

pub use kvs::*;

// Type

pub type SqlxError = Error<sqlx::Error>;

impl ErrorDetails for sqlx::Error {
  fn name(&self) -> &'static str {
    match self.as_database_error() {
      None => stringify!(SqlxError),
      Some(_) => stringify!(SqlxDatabaseError),
    }
  }

  fn details(&self) -> serde_json::Value {
    match self.as_database_error() {
      None => serde_json::Value::Null,
      Some(database) => serde_json::json!({
        "code": database.code(),
        "kind": format_args!("{:?}", database.kind()),
      }),
    }
  }
}

pub struct Database(SqlitePool);

impl AsRef<SqlitePool> for Database {
  fn as_ref(&self) -> &SqlitePool {
    &self.0
  }
}

impl Database {
  pub async fn new() -> Self {
    // Database storage folder
    //   In debug mode  : is in the src-tauri folder
    //   In release mode: Current executable folder
    let filename = if cfg!(debug_assertions) {
      PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(consts::DATABASE)
    } else {
      env::current_exe()
        .expect("Failed to get current executable path")
        .parent()
        .unwrap()
        .join(consts::DATABASE)
    };

    Self::new_with(filename).await
  }

  #[tracing::instrument]
  pub async fn new_with(filename: impl AsRef<Path> + Debug) -> Self {
    info!("Connecting to database...");
    let sqlite = SqlitePool::connect_with(
      SqliteConnectOptions::new()
        .filename(filename)
        .create_if_missing(true)
        .read_only(false)
        .immutable(false)
        .shared_cache(false),
    )
    .await
    .expect("Failed to connect database");

    let database = Self(sqlite);
    database
      .initialize()
      .await
      .expect("Failed to initialize database");

    database
  }

  #[tracing::instrument(skip(self))]
  pub async fn close(&self) {
    info!("Closing database...");
    self.0.close().await;
  }

  #[tracing::instrument(skip(self))]
  async fn initialize(&self) -> Result<(), sqlx::Error> {
    let version: u32 = self.0.fetch_one("PRAGMA USER_VERSION;").await?.get(0);
    let expected_version = SQLS.len();

    info!("Current database version: {version}, expected: {expected_version}");
    for sql in SQLS.iter().skip(version as _) {
      self.execute(sql).await?;
    }

    Ok(())
  }

  #[tracing::instrument(skip(self))]
  pub async fn execute(
    &self,
    query: impl AsRef<str> + fmt::Debug,
  ) -> Result<SqliteQueryResult, sqlx::Error> {
    let start = Instant::now();

    info!(message = "Executing database query");

    let ret = self.0.execute(query.as_ref()).await;

    info!(
      message = "Database query executed",
      elapsed = ?start.elapsed(),
      ?ret,
    );

    ret
  }

  /// Backup database to specified path
  /// This method will checkpoint the WAL file first, then copy the database file
  #[tracing::instrument(skip(self))]
  pub async fn backup(&self, backup_path: impl AsRef<Path> + Debug) -> Result<PathBuf, String> {
    let backup_path = backup_path.as_ref();

    info!(
      message = "Starting database backup",
      backup_path = ?backup_path,
    );

    let start = Instant::now();

    // Step 1: Checkpoint WAL to ensure all data is in the main database file
    self
      .0
      .execute("PRAGMA wal_checkpoint(TRUNCATE);")
      .await
      .map_err(|e| format!("Failed to checkpoint WAL: {e}"))?;

    // Step 2: Get the current database file path
    let db_path = self
      .0
      .connect_options()
      .get_filename()
      .to_path_buf();

    // Step 3: Copy the database file
    std::fs::copy(&db_path, backup_path)
      .map_err(|e| format!("Failed to copy database file: {e}"))?;

    info!(
      message = "Database backup completed",
      elapsed = ?start.elapsed(),
      source = ?db_path,
      backup_path = ?backup_path,
    );

    Ok(backup_path.to_path_buf())
  }

  /// Get the current database file path
  pub fn database_path(&self) -> PathBuf {
    self.0.connect_options().get_filename().to_path_buf()
  }

  /// Get the backup directory (same as database directory)
  fn backup_dir(&self) -> PathBuf {
    self
      .database_path()
      .parent()
      .expect("Database path should have a parent")
      .to_path_buf()
  }

  /// Create a backup with auto-incremented number
  /// Backup file format: {db_name}.bak.{number}
  #[tracing::instrument(skip(self))]
  pub async fn create_backup(&self) -> Result<BackupInfo, String> {
    let start = Instant::now();

    // Step 1: Checkpoint WAL
    self
      .0
      .execute("PRAGMA wal_checkpoint(TRUNCATE);")
      .await
      .map_err(|e| format!("Failed to checkpoint WAL: {e}"))?;

    // Step 2: Find next backup number
    let backup_number = self.get_next_backup_number();
    let db_path = self.database_path();
    let backup_path = self.get_backup_path(backup_number);

    // Step 3: Copy database file
    std::fs::copy(&db_path, &backup_path)
      .map_err(|e| format!("Failed to copy database file: {e}"))?;

    // Step 4: Get file metadata
    let metadata = std::fs::metadata(&backup_path)
      .map_err(|e| format!("Failed to get backup file metadata: {e}"))?;

    info!(
      message = "Database backup created",
      elapsed = ?start.elapsed(),
      backup_path = ?backup_path,
      backup_number,
    );

    Ok(BackupInfo {
      number: backup_number,
      path: backup_path,
      size: metadata.len(),
      modified_time: metadata
        .modified()
        .ok()
        .and_then(|t| t.elapsed().ok())
        .map(|d| d.as_secs()),
    })
  }

  /// Get the next available backup number
  fn get_next_backup_number(&self) -> u32 {
    let backup_dir = self.backup_dir();
    let db_path = self.database_path();
    let db_name = db_path
      .file_name()
      .and_then(|n| n.to_str())
      .unwrap_or("database.db");

    let prefix = format!("{db_name}.bak.");

    let mut max_number = 0u32;

    if let Ok(entries) = std::fs::read_dir(&backup_dir) {
      for entry in entries.flatten() {
        if let Some(name) = entry.file_name().to_str() {
          if name.starts_with(&prefix) {
            if let Some(number_str) = name.strip_prefix(&prefix) {
              if let Ok(number) = number_str.parse::<u32>() {
                max_number = max_number.max(number);
              }
            }
          }
        }
      }
    }

    max_number + 1
  }

  /// Get backup path for a given number
  fn get_backup_path(&self, number: u32) -> PathBuf {
    let db_path = self.database_path();
    let backup_name = format!("{}.bak.{}", db_path.file_name().unwrap().to_str().unwrap(), number);
    db_path.parent().unwrap().join(backup_name)
  }

  /// List all backup files
  pub fn list_backups(&self) -> Result<Vec<BackupInfo>, String> {
    let backup_dir = self.backup_dir();
    let db_path = self.database_path();
    let db_name = db_path
      .file_name()
      .and_then(|n| n.to_str())
      .unwrap_or("database.db");

    let prefix = format!("{db_name}.bak.");
    let mut backups = Vec::new();

    if let Ok(entries) = std::fs::read_dir(&backup_dir) {
      for entry in entries.flatten() {
        let path = entry.path();
        if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
          if name.starts_with(&prefix) {
            if let Some(number_str) = name.strip_prefix(&prefix) {
              if let Ok(number) = number_str.parse::<u32>() {
                if let Ok(metadata) = std::fs::metadata(&path) {
                  backups.push(BackupInfo {
                    number,
                    path: path.clone(),
                    size: metadata.len(),
                    modified_time: metadata
                      .modified()
                      .ok()
                      .and_then(|t| t.elapsed().ok())
                      .map(|d| d.as_secs()),
                  });
                }
              }
            }
          }
        }
      }
    }

    // Sort by number descending (newest first)
    backups.sort_by(|a, b| b.number.cmp(&a.number));

    Ok(backups)
  }

  /// Restore database from a backup file
  /// This will create a backup of current database before restoring
  /// Returns the backup number created before restore
  #[tracing::instrument(skip(self))]
  pub async fn restore_backup(&self, backup_number: u32) -> Result<u32, String> {
    let start = Instant::now();

    // Step 1: Checkpoint WAL
    self
      .0
      .execute("PRAGMA wal_checkpoint(TRUNCATE);")
      .await
      .map_err(|e| format!("Failed to checkpoint WAL: {e}"))?;

    // Step 2: Verify backup exists
    let backup_path = self.get_backup_path(backup_number);
    if !backup_path.exists() {
      return Err(format!("Backup file not found: {:?}", backup_path));
    }

    // Step 3: Create backup of current database before restore
    let pre_restore_backup_number = self.get_next_backup_number();
    let db_path = self.database_path();
    let pre_restore_backup_path = self.get_backup_path(pre_restore_backup_number);

    std::fs::copy(&db_path, &pre_restore_backup_path)
      .map_err(|e| format!("Failed to backup current database before restore: {e}"))?;

    info!(
      message = "Created pre-restore backup",
      pre_restore_backup_number,
      pre_restore_backup_path = ?pre_restore_backup_path,
    );

    // Step 4: Restore from backup
    std::fs::copy(&backup_path, &db_path)
      .map_err(|e| format!("Failed to restore database from backup: {e}"))?;

    info!(
      message = "Database restored from backup",
      elapsed = ?start.elapsed(),
      backup_number,
      restored_from = ?backup_path,
    );

    Ok(pre_restore_backup_number)
  }

  /// Delete a backup file
  pub fn delete_backup(&self, backup_number: u32) -> Result<(), String> {
    let backup_path = self.get_backup_path(backup_number);
    if backup_path.exists() {
      std::fs::remove_file(&backup_path)
        .map_err(|e| format!("Failed to delete backup file: {e}"))?;
      info!(message = "Backup deleted", backup_number, backup_path = ?backup_path);
    }
    Ok(())
  }
}

/// DatabaseManager provides atomic database switching capability
/// It wraps the database in a RwLock to allow hot-swapping during backup restore
pub struct DatabaseManager {
  inner: RwLock<Arc<Database>>,
  path: PathBuf,
}

impl DatabaseManager {
  pub fn new(database: Database) -> Self {
    let path = database.database_path();
    Self {
      inner: RwLock::new(Arc::new(database)),
      path,
    }
  }

  /// Get a read guard to the database
  pub async fn read(&self) -> tokio::sync::RwLockReadGuard<'_, Arc<Database>> {
    self.inner.read().await
  }

  /// Get the database file path
  pub fn database_path(&self) -> &Path {
    &self.path
  }

  /// Get the backup directory (same as database directory)
  fn backup_dir(&self) -> PathBuf {
    self
      .path
      .parent()
      .expect("Database path should have a parent")
      .to_path_buf()
  }

  /// Get backup path for a given number
  fn get_backup_path(number: u32) -> impl FnOnce(&Path) -> PathBuf {
    move |db_path: &Path| {
      let backup_name = format!("{}.bak.{}", db_path.file_name().unwrap().to_str().unwrap(), number);
      db_path.parent().unwrap().join(backup_name)
    }
  }

  /// Get the next available backup number
  fn get_next_backup_number(db_path: &Path) -> u32 {
    let backup_dir = db_path.parent().expect("Database path should have a parent");
    let db_name = db_path
      .file_name()
      .and_then(|n| n.to_str())
      .unwrap_or("database.db");

    let prefix = format!("{db_name}.bak.");

    let mut max_number = 0u32;

    if let Ok(entries) = std::fs::read_dir(backup_dir) {
      for entry in entries.flatten() {
        if let Some(name) = entry.file_name().to_str() {
          if name.starts_with(&prefix) {
            if let Some(number_str) = name.strip_prefix(&prefix) {
              if let Ok(number) = number_str.parse::<u32>() {
                max_number = max_number.max(number);
              }
            }
          }
        }
      }
    }

    max_number + 1
  }

  /// Restore database from backup and reconnect (hot-swap)
  /// This will:
  /// 1. Acquire write lock (blocking all reads)
  /// 2. Close old connection pool
  /// 3. Create backup of current database (for undo)
  /// 4. Replace database file with backup
  /// 5. Create new connection pool
  /// 6. Update the inner Arc
  ///
  /// Returns the backup number created before restore (for undo)
  #[tracing::instrument(skip(self))]
  pub async fn restore_and_reconnect(&self, backup_number: u32) -> Result<u32, String> {
    let start = Instant::now();

    // 1. Get write lock (blocks all reads)
    let mut db_guard = self.inner.write().await;

    // 2. Close old connection pool
    info!("Closing old database connection...");
    db_guard.close().await;

    // 3. Find backup file
    let backup_path = Self::get_backup_path(backup_number)(&self.path);
    if !backup_path.exists() {
      return Err(format!("Backup file not found: {:?}", backup_path));
    }

    // 4. Create backup of current database (for undo)
    let pre_restore_number = Self::get_next_backup_number(&self.path);
    let pre_restore_path = Self::get_backup_path(pre_restore_number)(&self.path);

    info!(
      message = "Creating pre-restore backup",
      pre_restore_number,
      pre_restore_path = ?pre_restore_path,
    );

    std::fs::copy(&self.path, &pre_restore_path)
      .map_err(|e| format!("Failed to backup current database: {e}"))?;

    // 5. Replace database file with backup
    std::fs::copy(&backup_path, &self.path)
      .map_err(|e| format!("Failed to restore database from backup: {e}"))?;

    // 6. Create new database connection
    info!("Creating new database connection...");
    let new_db = Database::new_with(&self.path).await;

    // 7. Update the inner Arc
    *db_guard = Arc::new(new_db);

    info!(
      message = "Database restored and reconnected",
      elapsed = ?start.elapsed(),
      backup_number,
    );

    Ok(pre_restore_number)
  }

  /// Create a backup with auto-incremented number
  #[tracing::instrument(skip(self))]
  pub async fn create_backup(&self) -> Result<BackupInfo, String> {
    let db = self.read().await;
    db.create_backup().await
  }

  /// List all backup files
  pub fn list_backups(&self) -> Result<Vec<BackupInfo>, String> {
    let backup_dir = self.backup_dir();
    let db_name = self
      .path
      .file_name()
      .and_then(|n| n.to_str())
      .unwrap_or("database.db");

    let prefix = format!("{db_name}.bak.");
    let mut backups = Vec::new();

    if let Ok(entries) = std::fs::read_dir(&backup_dir) {
      for entry in entries.flatten() {
        let path = entry.path();
        if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
          if name.starts_with(&prefix) {
            if let Some(number_str) = name.strip_prefix(&prefix) {
              if let Ok(number) = number_str.parse::<u32>() {
                if let Ok(metadata) = std::fs::metadata(&path) {
                  backups.push(BackupInfo {
                    number,
                    path: path.clone(),
                    size: metadata.len(),
                    modified_time: metadata
                      .modified()
                      .ok()
                      .and_then(|t| t.elapsed().ok())
                      .map(|d| d.as_secs()),
                  });
                }
              }
            }
          }
        }
      }
    }

    // Sort by number descending (newest first)
    backups.sort_by(|a, b| b.number.cmp(&a.number));

    Ok(backups)
  }

  /// Delete a backup file
  pub fn delete_backup(&self, backup_number: u32) -> Result<(), String> {
    let backup_path = Self::get_backup_path(backup_number)(&self.path);
    if backup_path.exists() {
      std::fs::remove_file(&backup_path)
        .map_err(|e| format!("Failed to delete backup file: {e}"))?;
      info!(message = "Backup deleted", backup_number, backup_path = ?backup_path);
    }
    Ok(())
  }
}

/// Backup file information
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupInfo {
  /// Backup number
  pub number: u32,
  /// Full path to backup file
  pub path: PathBuf,
  /// File size in bytes
  pub size: u64,
  /// Modified time as seconds ago from now
  pub modified_time: Option<u64>,
}

pub type DatabaseState<'r> = TauriState<'r, DatabaseManager>;

#[tauri::command]
pub async fn database_execute(
  database: DatabaseState<'_>,
  query: String,
) -> Result<u64, SqlxError> {
  let db = database.read().await;
  let ret = db.execute(query).await?;
  Ok(ret.rows_affected())
}

/// Create a backup with auto-incremented number
#[tauri::command]
#[tracing::instrument(skip_all)]
pub async fn database_create_backup(database: DatabaseState<'_>) -> Result<BackupInfo, String> {
  database.create_backup().await
}

/// List all backup files
#[tauri::command]
#[tracing::instrument(skip_all)]
pub fn database_list_backups(database: DatabaseState<'_>) -> Result<Vec<BackupInfo>, String> {
  database.list_backups()
}

/// Restore database from a backup file (hot-swap)
/// Returns the backup number created before restore (for undo)
#[tauri::command]
#[tracing::instrument(skip_all)]
pub async fn database_restore_backup(
  app: tauri::AppHandle,
  database: DatabaseState<'_>,
  backup_number: u32,
) -> Result<u32, String> {
  let result = database.restore_and_reconnect(backup_number).await?;

  // Emit event to notify frontend to refresh data
  app
    .emit(consts::EVENT_DATABASE_RESTORED, ())
    .map_err(|e| e.to_string())?;

  Ok(result)
}

/// Delete a backup file
#[tauri::command]
#[tracing::instrument(skip_all)]
pub fn database_delete_backup(database: DatabaseState<'_>, backup_number: u32) -> Result<(), String> {
  database.delete_backup(backup_number)
}

/// Get the current database file path
#[tauri::command]
#[tracing::instrument(skip_all)]
pub fn database_path(database: DatabaseState<'_>) -> PathBuf {
  database.database_path().to_path_buf()
}

// region: SQL

const SQL_V1: &str = r"
BEGIN TRANSACTION;

CREATE TABLE IF NOT EXISTS `HG_KVS` (
  `key`        TEXT NOT NULL PRIMARY KEY,
  `val`        TEXT NOT NULL,
  `updated_at` DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS `HG_ACCOUNTS` (
  `business`    INTEGER NOT NULL,
  `uid`         INTEGER NOT NULL,
  `data_folder` TEXT    NOT NULL,
  `properties`  TEXT,
  PRIMARY KEY (`business`, `uid`)
);
CREATE INDEX IF NOT EXISTS `HG_ACCOUNTS.business_idx` ON `HG_ACCOUNTS` (`business`);
CREATE INDEX IF NOT EXISTS `HG_ACCOUNTS.uid_idx`      ON `HG_ACCOUNTS` (`uid`);

CREATE TABLE IF NOT EXISTS `HG_GACHA_RECORDS` (
  `business`   INTEGER NOT NULL,
  `uid`        INTEGER NOT NULL,
  `id`         TEXT    NOT NULL,
  `gacha_type` INTEGER NOT NULL,
  `gacha_id`   INTEGER,
  `rank_type`  INTEGER NOT NULL,
  `count`      INTEGER NOT NULL,
  `time`       TEXT    NOT NULL,
  `lang`       TEXT    NOT NULL,
  `name`       TEXT    NOT NULL,
  `item_type`  TEXT    NOT NULL,
  `item_id`    TEXT    NOT NULL,
  PRIMARY KEY (`business`, `uid`, `id`)
);
CREATE INDEX IF NOT EXISTS `HG_GACHA_RECORDS.id_idx`                      ON `HG_GACHA_RECORDS` (`id`);
CREATE INDEX IF NOT EXISTS `HG_GACHA_RECORDS.gacha_type_idx`              ON `HG_GACHA_RECORDS` (`gacha_type`);
CREATE INDEX IF NOT EXISTS `HG_GACHA_RECORDS.rank_type_idx`               ON `HG_GACHA_RECORDS` (`rank_type`);
CREATE INDEX IF NOT EXISTS `HG_GACHA_RECORDS.business_uid_idx`            ON `HG_GACHA_RECORDS` (`business`, `uid`);
CREATE INDEX IF NOT EXISTS `HG_GACHA_RECORDS.business_uid_gacha_type_idx` ON `HG_GACHA_RECORDS` (`business`, `uid`, `gacha_type`);

PRAGMA USER_VERSION = 1;
COMMIT TRANSACTION;
";

// Changes:
// Table: `HG_GACHA_RECORDS`
// PK   : (business, uid, id)
//      : (business, uid, id, gacha_type)
//
// See  : https://github.com/lgou2w/HoYo.Gacha/issues/74

const SQL_V2: &str = r"
BEGIN TRANSACTION;
SAVEPOINT start_migration_v2;

ALTER TABLE `HG_GACHA_RECORDS` RENAME TO `HG_GACHA_RECORDS_OLD`;

CREATE TABLE IF NOT EXISTS `HG_GACHA_RECORDS` (
  `business`   INTEGER NOT NULL,
  `uid`        INTEGER NOT NULL,
  `id`         TEXT    NOT NULL,
  `gacha_type` INTEGER NOT NULL,
  `gacha_id`   INTEGER,
  `rank_type`  INTEGER NOT NULL,
  `count`      INTEGER NOT NULL,
  `time`       TEXT    NOT NULL,
  `lang`       TEXT    NOT NULL,
  `name`       TEXT    NOT NULL,
  `item_type`  TEXT    NOT NULL,
  `item_id`    TEXT    NOT NULL,
  PRIMARY KEY (`business`, `uid`, `id`, `gacha_type`)
);

INSERT INTO `HG_GACHA_RECORDS`
  (`business`, `uid`, `id`, `gacha_type`, `gacha_id`,
  `rank_type`, `count`, `time`, `lang`, `name`, `item_type`, `item_id`)
SELECT
  `business`, `uid`, `id`, `gacha_type`, `gacha_id`,
  `rank_type`, `count`, `time`, `lang`, `name`, `item_type`, `item_id`
FROM `HG_GACHA_RECORDS_OLD`;

DROP TABLE `HG_GACHA_RECORDS_OLD`;

CREATE INDEX IF NOT EXISTS `HG_GACHA_RECORDS.id_idx`                      ON `HG_GACHA_RECORDS` (`id`);
CREATE INDEX IF NOT EXISTS `HG_GACHA_RECORDS.gacha_type_idx`              ON `HG_GACHA_RECORDS` (`gacha_type`);
CREATE INDEX IF NOT EXISTS `HG_GACHA_RECORDS.rank_type_idx`               ON `HG_GACHA_RECORDS` (`rank_type`);
CREATE INDEX IF NOT EXISTS `HG_GACHA_RECORDS.business_uid_idx`            ON `HG_GACHA_RECORDS` (`business`, `uid`);
CREATE INDEX IF NOT EXISTS `HG_GACHA_RECORDS.business_uid_gacha_type_idx` ON `HG_GACHA_RECORDS` (`business`, `uid`, `gacha_type`);

PRAGMA USER_VERSION = 2;

RELEASE start_migration_v2;
COMMIT TRANSACTION;
";

const SQL_V3: &str = r"
BEGIN TRANSACTION;
SAVEPOINT start_migration_v3;

ALTER TABLE `HG_GACHA_RECORDS` ADD COLUMN `properties` TEXT DEFAULT NULL;

PRAGMA USER_VERSION = 3;

RELEASE start_migration_v3;
COMMIT TRANSACTION;
";

const SQLS: &[&str] = &[SQL_V1, SQL_V2, SQL_V3];

// endregion

// region: Questioner

#[allow(unused)]
type SqliteQuery = Query<'static, Sqlite, SqliteArguments<'static>>;
type SqliteQueryAs<T> = QueryAs<'static, Sqlite, T, SqliteArguments<'static>>;

pub trait Questioner {
  type Entity: Clone + DeserializeOwned + for<'r> FromRow<'r, SqliteRow> + Serialize + Sized;
}

macro_rules! declare_questioner {
  (
    $entity:ident,
    $(
      $sql:literal = $name:ident {
        $($arg_n:ident: $arg_t:ty,)*
      }: $operation:ident -> $result:ty,
    )*
  ) => {
    paste::paste! {
      pub struct [<$entity Questioner>];

      impl crate::database::Questioner for [<$entity Questioner>] {
        type Entity = $entity;
      }

      impl [<$entity Questioner>] {
        $(
          fn [<sql_ $name>](
            $($arg_n: $arg_t),*
          ) -> crate::database::SqliteQueryAs<<Self as crate::database::Questioner>::Entity> {
            sqlx::query_as($sql)
              $(.bind($arg_n))*
          }

          #[tracing::instrument(
            skip(database),
            fields(
              name = stringify!($name),
              operation = stringify!($operation),
            )
          )]
          pub async fn $name(
            database: &crate::database::Database,
            $($arg_n: $arg_t),*
          ) -> Result<$result, crate::database::SqlxError> {
            let start = Instant::now();

            tracing::info!(
              message = "Executing database operation",
              $($arg_n = ?$arg_n),*
            );

            let ret = Self::[<sql_ $name>]($($arg_n),*)
              .$operation(database.as_ref())
              .await
              .map_err(Into::into);

            tracing::info!(
              message = "Database operation executed",
              elapsed = ?start.elapsed(),
            );

            ret
          }
        )*
      }
    }
  };
}

macro_rules! declare_questioner_with_handlers {
  (
    $entity:ident,
    $(
      $sql:literal = $name:ident {
        $($arg_n:ident: $arg_t:ty,)*
      }: $operation:ident -> $result:ty,
    )*
  ) => {
    declare_questioner! {
      $entity,
      $(
        $sql = $name {
          $($arg_n: $arg_t,)*
        }: $operation -> $result,
      )*
    }

    paste::paste! {
      pub mod [<$entity:snake:lower _questioner>] {
        use super::*;

        $(
          #[tauri::command]
          pub async fn [<database_ $name>](
            database: crate::database::DatabaseState<'_>,
            $($arg_n: $arg_t),*
          ) -> Result<$result, crate::database::SqlxError> {
            let db = database.read().await;
            super::[<$entity Questioner>]::$name(&db, $($arg_n),*).await
          }
        )*
      }
    }
  }
}

// endregion

// region: Kv

declare_questioner_with_handlers! {
  Kv,

  "SELECT * FROM `HG_KVS` WHERE `key` = ?;"
    = find_kv { key: String, }: fetch_optional -> Option<Kv>,

  "INSERT INTO `HG_KVS` (`key`, `val`) VALUES (?, ?) RETURNING *;"
    = create_kv { key: String, val: String, }: fetch_one -> Kv,

  "UPDATE `HG_KVS` SET `val` = ?, `updated_at` = ? WHERE `key` = ? RETURNING *;"
    = update_kv {
      val: String,
      updated_at: Option<OffsetDateTime>,
      key: String,
    }: fetch_optional -> Option<Kv>,

  "INSERT OR REPLACE INTO `HG_KVS` (`key`, `val`, `updated_at`) VALUES (?, ?, ?) RETURNING *;"
    = upsert_kv {
      key: String,
      val: String,
      updated_at: Option<OffsetDateTime>,
    }: fetch_one -> Kv,

  "DELETE FROM `HG_KVS` WHERE `key` = ? RETURNING *;"
    = delete_kv { key: String, }: fetch_optional -> Option<Kv>,
}

impl<'r> FromRow<'r, SqliteRow> for Kv {
  fn from_row(row: &'r SqliteRow) -> Result<Self, sqlx::Error> {
    Ok(Self {
      key: row.try_get("key")?,
      val: row.try_get("val")?,
      updated_at: row.try_get("updated_at")?,
    })
  }
}

// endregion

// region: Account Questioner

declare_questioner_with_handlers! {
  Account,

  "SELECT * FROM `HG_ACCOUNTS` WHERE `business` = ?;"
    = find_accounts_by_business {
        business: Business,
      }: fetch_all -> Vec<Account>,

  "SELECT * FROM `HG_ACCOUNTS` WHERE `business` = ? AND `uid` = ?;"
    = find_account_by_business_and_uid {
        business: Business,
        uid: u32,
      }: fetch_optional -> Option<Account>,

  "INSERT INTO `HG_ACCOUNTS` (`business`, `uid`, `data_folder`, `properties`) VALUES (?, ?, ?, ?) RETURNING *;"
    = create_account {
        business: Business,
        uid: u32,
        data_folder: String,
        properties: Option<Properties>,
      }: fetch_one -> Account,

  "UPDATE `HG_ACCOUNTS` SET `data_folder` = ? WHERE `business` = ? AND `uid` = ? RETURNING *;"
    = update_account_data_folder_by_business_and_uid {
        data_folder: String,
        business: Business,
        uid: u32,
      }: fetch_optional -> Option<Account>,

  "UPDATE `HG_ACCOUNTS` SET `properties` = ? WHERE `business` = ? AND `uid` = ? RETURNING *;"
    = update_account_properties_by_business_and_uid {
        properties: Option<Properties>,
        business: Business,
        uid: u32,
      }: fetch_optional -> Option<Account>,

  "DELETE FROM `HG_ACCOUNTS` WHERE `business` = ? AND `uid` = ? RETURNING *;"
    = delete_account_by_business_and_uid {
        business: Business,
        uid: u32,
      }: fetch_optional -> Option<Account>,
}

impl<'r> FromRow<'r, SqliteRow> for Account {
  fn from_row(row: &'r SqliteRow) -> Result<Self, sqlx::Error> {
    Ok(Self {
      business: row.try_get("business")?,
      uid: row.try_get("uid")?,
      data_folder: row.try_get("data_folder")?,
      properties: row.try_get("properties")?,
    })
  }
}

impl Type<Sqlite> for Business {
  fn type_info() -> SqliteTypeInfo {
    u8::type_info()
  }

  fn compatible(ty: &SqliteTypeInfo) -> bool {
    u8::compatible(ty)
  }
}

impl<'r> Encode<'r, Sqlite> for Business {
  fn encode_by_ref(
    &self,
    buf: &mut <Sqlite as sqlx::Database>::ArgumentBuffer<'r>,
  ) -> Result<IsNull, BoxDynError> {
    u8::from(*self).encode_by_ref(buf)
  }
}

impl Decode<'_, Sqlite> for Business {
  fn decode(value: SqliteValueRef) -> Result<Self, BoxDynError> {
    Business::try_from(u8::decode(value)?).map_err(Into::into)
  }
}

impl Type<Sqlite> for Properties {
  fn type_info() -> SqliteTypeInfo {
    String::type_info()
  }

  fn compatible(ty: &SqliteTypeInfo) -> bool {
    String::compatible(ty)
  }
}

impl<'r> Encode<'r, Sqlite> for Properties {
  fn encode_by_ref(
    &self,
    buf: &mut <Sqlite as sqlx::Database>::ArgumentBuffer<'r>,
  ) -> Result<IsNull, BoxDynError> {
    serde_json::to_string(self)
      .map_err(|e| format!("Failed when serializing properties: {e}"))?
      .encode_by_ref(buf)
  }
}

impl Decode<'_, Sqlite> for Properties {
  fn decode(value: SqliteValueRef) -> Result<Self, BoxDynError> {
    serde_json::from_str(&String::decode(value)?)
      .map_err(|e| format!("Failed when deserializing properties: {e}").into())
  }
}

// endregion

// region: GachaRecord Questioner

declare_questioner_with_handlers! {
  GachaRecord,

  "SELECT * FROM `HG_GACHA_RECORDS` WHERE `uid` = ? ORDER BY `id` ASC;"
    = find_gacha_records_by_uid {
        uid: u32,
      }: fetch_all -> Vec<GachaRecord>,

  "SELECT * FROM `HG_GACHA_RECORDS` WHERE `business` = ? AND `uid` = ? ORDER BY `id` ASC;"
    = find_gacha_records_by_business_and_uid {
        business: Business,
        uid: u32,
      }: fetch_all -> Vec<GachaRecord>,

  "SELECT * FROM `HG_GACHA_RECORDS` WHERE `business` = ? AND `uid` = ? ORDER BY `id` ASC LIMIT ?;"
    = find_gacha_records_by_business_and_uid_with_limit {
        business: Business,
        uid: u32,
        limit: u32,
      }: fetch_all -> Vec<GachaRecord>,

  "SELECT * FROM `HG_GACHA_RECORDS` WHERE `business` = ? AND `uid` = ? AND `gacha_type` = ? ORDER BY `id` ASC;"
    = find_gacha_records_by_business_and_uid_with_gacha_type {
        business: Business,
        uid: u32,
        gacha_type: u32,
      }: fetch_all -> Vec<GachaRecord>,
}

impl<'r> FromRow<'r, SqliteRow> for GachaRecord {
  fn from_row(row: &'r SqliteRow) -> Result<Self, sqlx::Error> {
    Ok(Self {
      business: row.try_get("business")?,
      uid: row.try_get("uid")?,
      id: row.try_get("id")?,
      gacha_type: row.try_get("gacha_type")?,
      gacha_id: row.try_get("gacha_id")?,
      rank_type: row.try_get("rank_type")?,
      count: row.try_get("count")?,
      lang: row.try_get("lang")?,
      time: row.try_get("time")?,
      name: row.try_get("name")?,
      item_type: row.try_get("item_type")?,
      item_id: row
        .try_get::<String, _>("item_id")?
        .parse::<u32>()
        .map_err(|e| sqlx::Error::Decode(Box::new(e)))?,
      properties: row.try_get("properties")?,
    })
  }
}

#[derive(Copy, Clone, Debug, Deserialize)]
pub enum GachaRecordSaveOnConflict {
  Nothing,
  Update,
}

impl GachaRecordSaveOnConflict {
  fn sql(&self) -> &'static str {
    match *self {
      Self::Nothing => {
        "INSERT INTO `HG_GACHA_RECORDS` (
          `business`, `uid`, `id`, `gacha_type`, `gacha_id`, `rank_type`,
          `count`, `time`, `lang`, `name`, `item_type`, `item_id`,
          `properties`
        ) VALUES (
          ?, ?, ?, ?, ?, ?,
          ?, ?, ?, ?, ?, ?,
          ?
        ) ON CONFLICT (`business`, `uid`, `id`, `gacha_type`) DO NOTHING;"
      }
      Self::Update => {
        "INSERT INTO `HG_GACHA_RECORDS` (
          `business`, `uid`, `id`, `gacha_type`, `gacha_id`, `rank_type`,
          `count`, `time`, `lang`, `name`, `item_type`, `item_id`,
          `properties`
        ) VALUES (
          ?, ?, ?, ?, ?, ?,
          ?, ?, ?, ?, ?, ?,
          ?
        ) ON CONFLICT (`business`, `uid`, `id`, `gacha_type`) DO UPDATE SET
          `gacha_id`   = excluded.`gacha_id`,
          `rank_type`  = excluded.`rank_type`,
          `count`      = excluded.`count`,
          `time`       = excluded.`time`,
          `lang`       = excluded.`lang`,
          `name`       = excluded.`name`,
          `item_type`  = excluded.`item_type`,
          `item_id`    = excluded.`item_id`,
          `properties` = excluded.`properties`;"
      }
    }
  }
}

#[async_trait]
pub trait GachaRecordQuestionerAdditions {
  #[inline]
  fn sql_create_gacha_record(
    record: GachaRecord,
    save_on_conflict: GachaRecordSaveOnConflict,
  ) -> SqliteQuery {
    sqlx::query(save_on_conflict.sql())
      .bind(record.business)
      .bind(record.uid)
      .bind(record.id)
      .bind(record.gacha_type)
      .bind(record.gacha_id)
      .bind(record.rank_type)
      .bind(record.count)
      .bind(record.time)
      .bind(record.lang)
      .bind(record.name)
      .bind(record.item_type)
      .bind(record.item_id)
      .bind(record.properties)
  }

  #[tracing::instrument(skip(database, records, progress_reporter), fields(records = records.len()))]
  async fn create_gacha_records(
    database: &Database,
    records: Vec<GachaRecord>,
    save_on_conflict: GachaRecordSaveOnConflict,
    progress_reporter: Option<mpsc::Sender<f32>>,
  ) -> Result<u64, SqlxError> {
    info!("Executing create gacha records database operation...");
    let total = records.len();
    let start = Instant::now();

    let mut txn = database.as_ref().begin().await?;
    let mut changes = 0;
    let mut completes = 0;
    let mut last_progress_reported = Instant::now();

    for record in records {
      completes += 1;
      changes +=
        <GachaRecordQuestioner as GachaRecordQuestionerAdditions>::sql_create_gacha_record(
          record,
          save_on_conflict,
        )
        .execute(&mut *txn)
        .await?
        .rows_affected();

      // Progress reporting: 200ms interval
      // Avoiding excessive recording leading to frequent reporting
      if let Some(reporter) = &progress_reporter {
        if last_progress_reported.elapsed().as_millis() > 200 {
          last_progress_reported = Instant::now();

          let progress = completes as f32 / total as f32;
          let progress = (progress * 100.).round() / 100.;
          if progress > 0. {
            let _ = reporter.try_send(progress);
          }
        }
      }
    }
    txn.commit().await?;

    // Avoiding incomplete progress due to reporting intervals
    let _ = progress_reporter.map(|reporter| reporter.try_send(1.0));

    info!(
      message = "Creation of gacha records completed",
      changes = ?changes,
      elapsed = ?start.elapsed()
    );

    Ok(changes)
  }

  #[tracing::instrument(skip(database))]
  async fn delete_gacha_records_by_business_and_uid(
    database: &Database,
    business: Business,
    uid: u32,
  ) -> Result<u64, SqlxError> {
    info!("Executing delete gacha records database operation...");
    let start = Instant::now();
    let changes = sqlx::query("DELETE FROM `HG_GACHA_RECORDS` WHERE `business` = ? AND `uid` = ?;")
      .bind(business)
      .bind(uid)
      .execute(database.as_ref())
      .await?
      .rows_affected();

    info!(
      message = "Deletion of gacha records completed",
      changes = ?changes,
      elapsed = ?start.elapsed(),
    );

    Ok(changes)
  }

  #[tracing::instrument(skip(database))]
  async fn delete_gacha_records_by_newer_than_end_id(
    database: &Database,
    business: Business,
    uid: u32,
    gacha_type: u32,
    end_id: &str,
  ) -> Result<u64, SqlxError> {
    info!("Executing delete gacha records by newer than end_id database operation...");
    let start = Instant::now();
    let changes = sqlx::query("DELETE FROM `HG_GACHA_RECORDS` WHERE `business` = ? AND `uid` = ? AND `gacha_type` = ? AND `id` >= ?;")
      .bind(business)
      .bind(uid)
      .bind(gacha_type)
      .bind(end_id)
      .execute(database.as_ref())
      .await?
      .rows_affected();

    info!(
      message = "Deletion of gacha records by newer than end_id completed",
      changes = ?changes,
      elapsed = ?start.elapsed(),
    );

    Ok(changes)
  }

  #[tracing::instrument(skip(database))]
  async fn find_gacha_records_by_businesses_and_uid(
    database: &Database,
    businesses: &HashSet<Business>,
    uid: u32,
  ) -> Result<Vec<GachaRecord>, SqlxError> {
    info!("Executing find gacha records by businesses and uid database operation...");
    let start = Instant::now();
    let records = sqlx::query_as(
      "SELECT * FROM `HG_GACHA_RECORDS` WHERE `business` IN (?) AND `uid` = ? ORDER BY `id` ASC;",
    )
    .bind(
      businesses
        .iter()
        .map(|b| (*b as u8).to_string())
        .collect::<Vec<_>>()
        .join(","),
    )
    .bind(uid)
    .fetch_all(database.as_ref())
    .await?;

    info!(
      message = "Finding of gacha records completed",
      records = ?records.len(),
      elapsed = ?start.elapsed(),
    );

    Ok(records)
  }

  #[tracing::instrument(skip(database))]
  async fn find_gacha_records_by_businesses_or_uid(
    database: &Database,
    businesses: Option<&HashSet<Business>>,
    uid: u32,
  ) -> Result<Vec<GachaRecord>, SqlxError> {
    if let Some(businesses) = businesses {
      Self::find_gacha_records_by_businesses_and_uid(database, businesses, uid).await
    } else {
      GachaRecordQuestioner::find_gacha_records_by_uid(database, uid).await
    }
  }
}

impl GachaRecordQuestionerAdditions for GachaRecordQuestioner {}

pub mod gacha_record_questioner_additions {
  use super::*;

  #[tauri::command]
  pub async fn database_create_gacha_records(
    database: DatabaseState<'_>,
    records: Vec<GachaRecord>,
    on_conflict: GachaRecordSaveOnConflict,
  ) -> Result<u64, SqlxError> {
    let db = database.read().await;
    GachaRecordQuestioner::create_gacha_records(&db, records, on_conflict, None).await
  }

  #[tauri::command]
  pub async fn database_delete_gacha_records_by_business_and_uid(
    database: DatabaseState<'_>,
    business: Business,
    uid: u32,
  ) -> Result<u64, SqlxError> {
    let db = database.read().await;
    GachaRecordQuestioner::delete_gacha_records_by_business_and_uid(
      &db,
      business,
      uid,
    )
    .await
  }

  #[tauri::command]
  pub async fn database_find_gacha_records_by_businesses_and_uid(
    database: DatabaseState<'_>,
    businesses: HashSet<Business>,
    uid: u32,
  ) -> Result<Vec<GachaRecord>, SqlxError> {
    let db = database.read().await;
    GachaRecordQuestioner::find_gacha_records_by_businesses_and_uid(
      &db,
      &businesses,
      uid,
    )
    .await
  }

  #[tauri::command]
  pub async fn database_find_gacha_records_by_businesses_or_uid(
    database: DatabaseState<'_>,
    businesses: Option<HashSet<Business>>,
    uid: u32,
  ) -> Result<Vec<GachaRecord>, SqlxError> {
    let db = database.read().await;
    GachaRecordQuestioner::find_gacha_records_by_businesses_or_uid(
      &db,
      businesses.as_ref(),
      uid,
    )
    .await
  }
}

#[tauri::command]
#[tracing::instrument(skip_all)]
pub async fn database_legacy_migration(
  database: DatabaseState<'_>,
  legacy_database: Option<PathBuf>,
) -> Result<MigrationMetrics, LegacyMigrationError> {
  let db = database.read().await;
  if let Some(legacy_database) = legacy_database {
    legacy_migration::migration_with(&db, legacy_database).await
  } else {
    legacy_migration::migration(&db).await
  }
}

// endregion

#[cfg(test)]
mod tests {
  use super::*;
  use crate::error::SERIALIZATION_MARKER;

  #[test]
  fn test_error_serialize() {
    let error = SqlxError::from(sqlx::Error::ColumnNotFound("column".into()));

    assert_eq!(format!("{error:?}"), "Error(ColumnNotFound(\"column\"))");
    assert_eq!(
      serde_json::to_string(&error).unwrap(),
      format!(
        r#"{{"name":"{name}","message":"{error}","details":null,"{SERIALIZATION_MARKER}":true}}"#,
        name = error.as_ref().name()
      )
    );
  }
}
