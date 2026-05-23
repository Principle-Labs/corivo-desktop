use std::path::PathBuf;

use serde::Serialize;

use crate::error::Result;

pub mod migrations;
pub mod pool;
pub mod repos;
pub mod time;

use pool::{create_pool, run_blocking, DbPool};

#[derive(Clone)]
pub struct Database {
    pool: DbPool,
    db_path: PathBuf,
}

impl Database {
    pub fn initialize(data_dir: PathBuf) -> Result<Self> {
        std::fs::create_dir_all(&data_dir)?;

        let db_path = data_dir.join("corivo.sqlite");
        let pool = create_pool(&db_path)?;
        let conn = pool.get()?;
        migrations::apply_migrations(&conn)?;

        Ok(Self { pool, db_path })
    }

    pub fn pool(&self) -> DbPool {
        self.pool.clone()
    }

    pub fn db_path(&self) -> &PathBuf {
        &self.db_path
    }

    pub async fn get_stats(&self) -> Result<DbStats> {
        let db_path = self.db_path.clone();
        run_blocking(self.pool.clone(), move |conn| {
            let frames_count = conn
                .query_row("SELECT COUNT(*) FROM frames", [], |row| row.get(0))
                .unwrap_or(0i64);
            let file_size_bytes = std::fs::metadata(&db_path)
                .map(|metadata| metadata.len() as i64)
                .unwrap_or(0);

            Ok(DbStats {
                file_size_bytes,
                frames_count,
            })
        })
        .await
    }

    pub fn shutdown(&self) -> Result<()> {
        let conn = self.pool.get()?;
        conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")?;
        Ok(())
    }

    /// Drop every Corivo table and re-apply `schema.sql` from scratch.
    /// Used by the "清空所有数据" settings action — strict wipe, no
    /// preservation. The boot migration in `apply_migrations()` runs the
    /// same purge but wraps it with chat-thread preservation so a
    /// schema bump doesn't blow away the user's named conversations;
    /// here we want the opposite, so we call the bare purge directly.
    ///
    /// Caller is responsible for stopping the capture pipeline first
    /// and for wiping the on-disk screenshot store; this method is
    /// strictly the SQL side.
    pub async fn wipe_and_rebuild(&self) -> Result<()> {
        run_blocking(self.pool.clone(), |conn| {
            migrations::purge_legacy_and_apply_new(&conn)
        })
        .await
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct DbStats {
    pub file_size_bytes: i64,
    pub frames_count: i64,
}
