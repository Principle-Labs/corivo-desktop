use std::path::Path;

use r2d2::Pool;
use r2d2_sqlite::SqliteConnectionManager;

use crate::error::{CorivoError, Result};

pub type DbPool = Pool<SqliteConnectionManager>;
pub type DbConnection = r2d2::PooledConnection<SqliteConnectionManager>;

pub fn create_pool(db_path: &Path) -> Result<DbPool> {
    let manager = SqliteConnectionManager::file(db_path).with_init(|connection| {
        connection.execute_batch(
            r#"
PRAGMA journal_mode = WAL;
PRAGMA synchronous = NORMAL;
PRAGMA foreign_keys = ON;
PRAGMA busy_timeout = 5000;
PRAGMA temp_store = MEMORY;
"#,
        )?;
        Ok(())
    });

    Pool::builder()
        .max_size(8)
        .build(manager)
        .map_err(|error| CorivoError::Internal(format!("创建连接池失败: {error}")))
}

/// Test-only helper. DO NOT USE IN PRODUCTION CODE.
///
/// Build a pool backed by its own isolated in-memory SQLite database.
///
/// Each call returns a pool whose URI is keyed on a fresh `Uuid::new_v4()`, so
/// two pools constructed in the same process (e.g. two integration tests
/// running in parallel) do **not** share schema or rows. Inter-pool isolation
/// is therefore guaranteed by the per-call UUID; intra-pool visibility of
/// migrations is guaranteed by `max_size(1)` plus `cache=shared` on the URI
/// (all connections from the same pool see each other's writes because the
/// pool hands out the same single slot and shared cache keeps the single
/// in-memory database alive while the pool is held).
///
/// Only intended for tests — production code paths should use `create_pool`.
#[doc(hidden)]
pub fn test_in_memory_pool() -> Result<DbPool> {
    use rusqlite::OpenFlags;
    use uuid::Uuid;

    let uri = format!(
        "file:corivo-test-{}?mode=memory&cache=shared",
        Uuid::new_v4()
    );
    let manager = SqliteConnectionManager::file(uri)
        .with_flags(
            OpenFlags::SQLITE_OPEN_READ_WRITE
                | OpenFlags::SQLITE_OPEN_CREATE
                | OpenFlags::SQLITE_OPEN_URI
                | OpenFlags::SQLITE_OPEN_SHARED_CACHE,
        )
        .with_init(|connection| {
            connection.execute_batch(
                r#"
PRAGMA foreign_keys = ON;
PRAGMA busy_timeout = 5000;
PRAGMA temp_store = MEMORY;
"#,
            )?;
            Ok(())
        });

    Pool::builder()
        .max_size(1)
        .build(manager)
        .map_err(|error| CorivoError::Internal(format!("创建测试连接池失败: {error}")))
}

pub async fn run_blocking<F, T>(pool: DbPool, operation: F) -> Result<T>
where
    F: FnOnce(DbConnection) -> Result<T> + Send + 'static,
    T: Send + 'static,
{
    tokio::task::spawn_blocking(move || {
        let connection = pool
            .get()
            .map_err(|error| CorivoError::Internal(format!("获取数据库连接失败: {error}")))?;
        operation(connection)
    })
    .await
    .map_err(|error| CorivoError::Internal(format!("blocking task panic: {error}")))?
}
