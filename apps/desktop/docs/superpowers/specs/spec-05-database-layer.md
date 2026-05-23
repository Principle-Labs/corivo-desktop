# spec-05-database-layer.md

## 一、目标

为 Corivo 实现完整的本地 SQLite 持久层。完成本 spec 后：

SQLite 数据库初始化、路径管理、连接池、WAL 模式配置到位
Schema migrations 机制就绪，未来加字段加表不需要手动改库
三张核心表（screenshots / sessions / segments）+ 两张 P1 预留表（extracted_facts / conflict_alerts）建好
每张表有一个对应的 repo 层，封装所有 SQL 操作
支持应用启动时自动建库、版本迁移、健康检查
完整单元测试覆盖

核心原则：spec-05 只做数据层，不碰业务逻辑。CaptureLoop / SegmentService / MemoryService 都在后续 spec 里接入这一层。

## 二、不做什么

- ❌ 不实现截图采集（spec-06 做）
- ❌ 不实现 SegmentService / SummaryService（spec-07 做）
- ❌ 不接入 MVP pipeline（spec-mvp-loop 的内存实现先保留，后续再迁）
- ❌ 不做 SQL ORM（直接裸 rusqlite，够用）
- ❌ 不做全文搜索（Supermemory 已经负责这部分）
- ❌ 不做分布式 / 多设备同步
- ❌ 不做数据库备份 / 导出（spec-11 存储子页再做）
- ❌ 不做软删除（直接 DELETE）

## 三、成功标准

启动 app，应用数据目录下自动生成 corivo.sqlite 和 corivo.sqlite-wal
schema_version 表有一条 version=1 的记录
所有 repo 的单元测试通过（用临时目录的 SQLite 实例）
手动插入数据后重启 app，数据仍然存在
调用 get_db_stats command 能返回数据库大小和各表行数
连接池支持多个 tokio task 并发访问不报错
关闭 app 时不留下 .sqlite-wal 超大文件（正常 checkpoint）

## 四、技术决策

项选型理由
- SQLite 绑定：rusqlite (bundled)，Rust 生态主流，bundled feature 避免系统 SQLite 版本差异
- 连接池：r2d2 + r2d2_sqlite，成熟、轻量，支持 WAL 并发
- 异步封装：tokio 的 spawn_blocking，rusqlite 本身是同步的，包一层足够
- 日期时间：存 RFC3339 文本，人类可读、排序正确、时区明确
- Migrations：手写 + version 表，P0 表结构简单，不需要 refinery/sqlx-migrate
- JSON 字段：存 TEXT，反序列化到 serde_json::Value，SQLite 的 JSON 能力够用，没必要上 JSON1 扩展

## 五、新增依赖

src-tauri/Cargo.toml：

toml
rusqlite = { version = "0.31", features = ["bundled", "chrono", "serde_json"] }
r2d2 = "0.8"
r2d2_sqlite = "0.24"

## 六、目录结构

src-tauri/src/
├── db/
│   ├── mod.rs              Database 结构 + 初始化
│   ├── pool.rs             连接池封装
│   ├── migrations.rs       schema 版本迁移
│   ├── schema.sql          全量建表 SQL（version 1）
│   ├── types.rs            DB 层通用类型（DateTime 转换辅助等）
│   └── repos/
│       ├── mod.rs
│       ├── screenshots.rs
│       ├── sessions.rs
│       ├── segments.rs
│       ├── extracted_facts.rs   P0 建表但不用，P1 用
│       └── conflict_alerts.rs   P0 建表但不用，P1 用
├── commands/
│   └── db_debug.rs         调试用 command：查 DB 统计

## 七、数据库初始化

### 7.1 schema.sql（全量建表）

src-tauri/src/db/schema.sql：
sql-- ============================================
-- Corivo SQLite Schema - Version 1
-- ============================================

-- 会话表
CREATE TABLE IF NOT EXISTS sessions (
id TEXT PRIMARY KEY,
started_at TEXT NOT NULL,
ended_at TEXT,
interval_secs INTEGER NOT NULL,
screenshot_count INTEGER NOT NULL DEFAULT 0,
status TEXT NOT NULL DEFAULT 'active'  -- active / completed / aborted
);
CREATE INDEX IF NOT EXISTS idx_sessions_started_at ON sessions(started_at DESC);
CREATE INDEX IF NOT EXISTS idx_sessions_status ON sessions(status);

-- 截图表
CREATE TABLE IF NOT EXISTS screenshots (
id INTEGER PRIMARY KEY AUTOINCREMENT,
session_id TEXT NOT NULL,
captured_at TEXT NOT NULL,
file_path TEXT NOT NULL,
file_size INTEGER NOT NULL,
width INTEGER,
height INTEGER,
segment_id INTEGER,
FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE,
FOREIGN KEY (segment_id) REFERENCES segments(id) ON DELETE SET NULL
);
CREATE INDEX IF NOT EXISTS idx_screenshots_captured_at ON screenshots(captured_at DESC);
CREATE INDEX IF NOT EXISTS idx_screenshots_session_id ON screenshots(session_id);
CREATE INDEX IF NOT EXISTS idx_screenshots_segment_id ON screenshots(segment_id);

-- Segment 表（总结单元）
CREATE TABLE IF NOT EXISTS segments (
id INTEGER PRIMARY KEY AUTOINCREMENT,
session_id TEXT,
started_at TEXT NOT NULL,
ended_at TEXT NOT NULL,
status TEXT NOT NULL,  -- pending / processing / done / failed / partial_done
prompt_used TEXT,
summary TEXT,
activity_type TEXT,    -- 预留字段，P0 不填
model TEXT,
input_tokens INTEGER,
output_tokens INTEGER,
cost_usd REAL,
error_message TEXT,
generated_at TEXT,
memory_id TEXT,        -- 写入 Supermemory 后的 ID
screenshot_count INTEGER NOT NULL DEFAULT 0,
created_at TEXT NOT NULL DEFAULT (datetime('now')),
FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE SET NULL
);
CREATE INDEX IF NOT EXISTS idx_segments_started_at ON segments(started_at DESC);
CREATE INDEX IF NOT EXISTS idx_segments_status ON segments(status);
CREATE INDEX IF NOT EXISTS idx_segments_session_id ON segments(session_id);

-- P1 预留表：实时抽取的事实
CREATE TABLE IF NOT EXISTS extracted_facts (
id INTEGER PRIMARY KEY AUTOINCREMENT,
extracted_at TEXT NOT NULL,
source_screenshot_id INTEGER,
fact_type TEXT NOT NULL,
payload TEXT NOT NULL,
subject TEXT,
start_time TEXT,
end_time TEXT,
confidence REAL,
status TEXT DEFAULT 'active',
superseded_by INTEGER,
created_at TEXT NOT NULL DEFAULT (datetime('now')),
FOREIGN KEY (source_screenshot_id) REFERENCES screenshots(id) ON DELETE SET NULL,
FOREIGN KEY (superseded_by) REFERENCES extracted_facts(id) ON DELETE SET NULL
);
CREATE INDEX IF NOT EXISTS idx_facts_time_range ON extracted_facts(start_time, end_time);
CREATE INDEX IF NOT EXISTS idx_facts_status ON extracted_facts(status);
CREATE INDEX IF NOT EXISTS idx_facts_type ON extracted_facts(fact_type);

-- P1 预留表：冲突告警
CREATE TABLE IF NOT EXISTS conflict_alerts (
id INTEGER PRIMARY KEY AUTOINCREMENT,
detected_at TEXT NOT NULL,
fact_a_id INTEGER NOT NULL,
fact_b_id INTEGER NOT NULL,
conflict_type TEXT NOT NULL,
notified INTEGER DEFAULT 0,
user_feedback TEXT,
feedback_at TEXT,
FOREIGN KEY (fact_a_id) REFERENCES extracted_facts(id) ON DELETE CASCADE,
FOREIGN KEY (fact_b_id) REFERENCES extracted_facts(id) ON DELETE CASCADE
);

-- Schema 版本表
CREATE TABLE IF NOT EXISTS schema_version (
version INTEGER PRIMARY KEY,
applied_at TEXT NOT NULL DEFAULT (datetime('now'))
);
INSERT OR IGNORE INTO schema_version (version) VALUES (1);

### 7.2 连接池封装

src-tauri/src/db/pool.rs：
rustuse crate::error::{CorivoError, Result};
use r2d2::Pool;
use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::Connection;
use std::path::Path;

pub type DbPool = Pool<SqliteConnectionManager>;
pub type DbConnection = r2d2::PooledConnection<SqliteConnectionManager>;

/// 创建连接池并配置 WAL + 外键 + busy_timeout
pub fn create_pool(db_path: &Path) -> Result<DbPool> {
let manager = SqliteConnectionManager::file(db_path).with_init(|conn| {
conn.execute_batch(
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

    let pool = Pool::builder()
        .max_size(8)  // 8 个连接够了，多了也没用
        .build(manager)
        .map_err(|e| CorivoError::Internal(format!("创建连接池失败: {}", e)))?;

    Ok(pool)
}

/// 便捷函数：在 tokio 异步上下文里执行同步 SQL 操作
pub async fn run_blocking<F, T>(pool: DbPool, f: F) -> Result<T>
where
F: FnOnce(DbConnection) -> Result<T> + Send + 'static,
T: Send + 'static,
{
tokio::task::spawn_blocking(move || {
let conn = pool
.get()
.map_err(|e| CorivoError::Internal(format!("获取数据库连接失败: {}", e)))?;
f(conn)
})
.await
.map_err(|e| CorivoError::Internal(format!("blocking task panic: {}", e)))?
}
关键点：

WAL 模式：多个读连接 + 一个写连接可以并发，不阻塞
busy_timeout=5000：5 秒内重试，避免偶发的"database is locked"
foreign_keys=ON：SQLite 默认关闭外键，必须显式打开
run_blocking 工具函数：所有 rusqlite 操作都同步，用这个包一层让业务层能 await

### 7.3 Migrations

src-tauri/src/db/migrations.rs：
rustuse crate::db::pool::DbConnection;
use crate::error::{CorivoError, Result};

const SCHEMA_V1: &str = include_str!("schema.sql");

/// 当前代码所支持的最新 schema 版本
pub const LATEST_VERSION: i64 = 1;

/// 未来的迁移会是这样的结构:
///
/// ```rust
/// const MIGRATION_V2: &str = include_str!("migrations/v2_add_tags.sql");
///
/// fn apply_v2(conn: &mut Connection) -> Result<()> {
///     conn.execute_batch(MIGRATION_V2)?;
///     Ok(())
/// }
/// ```

pub fn apply_migrations(conn: &mut DbConnection) -> Result<()> {
let current_version = get_current_version(conn)?;

    if current_version >= LATEST_VERSION {
        tracing::info!("DB schema version {} is up to date", current_version);
        return Ok(());
    }

    tracing::info!(
        "DB schema migration needed: v{} -> v{}",
        current_version,
        LATEST_VERSION
    );

    // V0 → V1: 初次建库，跑全量 schema
    if current_version < 1 {
        conn.execute_batch(SCHEMA_V1)
            .map_err(|e| CorivoError::Internal(format!("V1 migration failed: {}", e)))?;
        tracing::info!("Applied V1 schema");
    }

    // 未来版本的迁移在这里加:
    // if current_version < 2 {
    //     apply_v2(conn)?;
    //     record_version(conn, 2)?;
    // }

    Ok(())
}

fn get_current_version(conn: &DbConnection) -> Result<i64> {
// schema_version 表可能还不存在（全新库）
let table_exists: bool = conn
.query_row(
"SELECT EXISTS (SELECT 1 FROM sqlite_master WHERE type='table' AND name='schema_version')",
[],
|row| row.get::<_, i32>(0).map(|v| v == 1),
)
.map_err(|e| CorivoError::Internal(format!("检查 schema_version 表失败: {}", e)))?;

    if !table_exists {
        return Ok(0);
    }

    let version: Option<i64> = conn
        .query_row(
            "SELECT MAX(version) FROM schema_version",
            [],
            |row| row.get(0),
        )
        .map_err(|e| CorivoError::Internal(format!("读取 schema_version 失败: {}", e)))?;

    Ok(version.unwrap_or(0))
}

#[allow(dead_code)]
fn record_version(conn: &DbConnection, version: i64) -> Result<()> {
conn.execute(
"INSERT INTO schema_version (version) VALUES (?1)",
[version],
)
.map_err(|e| CorivoError::Internal(format!("记录 schema 版本失败: {}", e)))?;
Ok(())
}

### 7.4 Database 顶层结构

src-tauri/src/db/mod.rs：
rustuse crate::error::{CorivoError, Result};
use std::path::PathBuf;
use std::sync::Arc;

pub mod migrations;
pub mod pool;
pub mod repos;
pub mod types;

use pool::{create_pool, DbPool};

/// 数据库门面对象，持有连接池，分发给各个 repo
#[derive(Clone)]
pub struct Database {
pool: DbPool,
db_path: PathBuf,
}

impl Database {
/// 初始化数据库：建路径、创建连接池、跑 migration
pub fn initialize(data_dir: PathBuf) -> Result<Self> {
// 确保目录存在
std::fs::create_dir_all(&data_dir)
.map_err(|e| CorivoError::Internal(format!("创建数据目录失败: {}", e)))?;

        let db_path = data_dir.join("corivo.sqlite");
        tracing::info!("initializing database at {:?}", db_path);

        let pool = create_pool(&db_path)?;

        // 跑 migration
        {
            let mut conn = pool
                .get()
                .map_err(|e| CorivoError::Internal(format!("获取初始连接失败: {}", e)))?;
            migrations::apply_migrations(&mut conn)?;
        }

        Ok(Self { pool, db_path })
    }

    pub fn pool(&self) -> DbPool {
        self.pool.clone()
    }

    pub fn db_path(&self) -> &PathBuf {
        &self.db_path
    }

    /// 返回 repo 工厂，方便业务层调用
    pub fn screenshots(&self) -> repos::screenshots::ScreenshotsRepo {
        repos::screenshots::ScreenshotsRepo::new(self.pool.clone())
    }

    pub fn sessions(&self) -> repos::sessions::SessionsRepo {
        repos::sessions::SessionsRepo::new(self.pool.clone())
    }

    pub fn segments(&self) -> repos::segments::SegmentsRepo {
        repos::segments::SegmentsRepo::new(self.pool.clone())
    }

    /// 调试：获取数据库统计信息
    pub async fn get_stats(&self) -> Result<DbStats> {
        let pool = self.pool.clone();
        let db_path = self.db_path.clone();

        pool::run_blocking(pool, move |conn| {
            let screenshots_count: i64 = conn.query_row(
                "SELECT COUNT(*) FROM screenshots",
                [],
                |r| r.get(0),
            )?;
            let sessions_count: i64 = conn.query_row(
                "SELECT COUNT(*) FROM sessions",
                [],
                |r| r.get(0),
            )?;
            let segments_count: i64 = conn.query_row(
                "SELECT COUNT(*) FROM segments",
                [],
                |r| r.get(0),
            )?;

            let file_size = std::fs::metadata(&db_path)
                .map(|m| m.len() as i64)
                .unwrap_or(0);

            Ok(DbStats {
                file_size_bytes: file_size,
                screenshots_count,
                sessions_count,
                segments_count,
            })
        })
        .await
    }

    /// 应用关闭前调用，做 WAL checkpoint
    pub fn shutdown(&self) -> Result<()> {
        let mut conn = self
            .pool
            .get()
            .map_err(|e| CorivoError::Internal(format!("shutdown 获取连接失败: {}", e)))?;
        conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")
            .map_err(|e| CorivoError::Internal(format!("WAL checkpoint 失败: {}", e)))?;
        tracing::info!("database shutdown, WAL truncated");
        Ok(())
    }
}

#[derive(Debug, serde::Serialize)]
pub struct DbStats {
pub file_size_bytes: i64,
pub screenshots_count: i64,
pub sessions_count: i64,
pub segments_count: i64,
}

### 7.5 通用类型辅助

src-tauri/src/db/types.rs：
rustuse chrono::{DateTime, Utc};
use rusqlite::{types::FromSql, types::FromSqlResult, types::ValueRef, ToSql, Result as RusqliteResult};

/// DateTime<Utc> 与 TEXT 的双向转换
/// rusqlite 的 chrono feature 已经支持，但显式封装便于未来修改
pub fn datetime_to_sql(dt: &DateTime<Utc>) -> String {
dt.to_rfc3339()
}

pub fn sql_to_datetime(s: &str) -> Result<DateTime<Utc>, chrono::ParseError> {
DateTime::parse_from_rfc3339(s).map(|dt| dt.with_timezone(&Utc))
}

## 八、Repo 实现

### 8.1 SessionsRepo

src-tauri/src/db/repos/sessions.rs：
rustuse crate::db::pool::{run_blocking, DbPool};
use crate::error::{CorivoError, Result};
use chrono::{DateTime, Utc};
use rusqlite::params;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
pub id: String,
pub started_at: DateTime<Utc>,
pub ended_at: Option<DateTime<Utc>>,
pub interval_secs: i64,
pub screenshot_count: i64,
pub status: String,
}

#[derive(Clone)]
pub struct SessionsRepo {
pool: DbPool,
}

impl SessionsRepo {
pub fn new(pool: DbPool) -> Self {
Self { pool }
}

    pub async fn create(&self, id: String, interval_secs: i64) -> Result<Session> {
        let now = Utc::now();
        let session = Session {
            id: id.clone(),
            started_at: now,
            ended_at: None,
            interval_secs,
            screenshot_count: 0,
            status: "active".to_string(),
        };

        let s = session.clone();
        run_blocking(self.pool.clone(), move |conn| {
            conn.execute(
                "INSERT INTO sessions (id, started_at, interval_secs, screenshot_count, status)
                 VALUES (?1, ?2, ?3, 0, 'active')",
                params![s.id, s.started_at.to_rfc3339(), s.interval_secs],
            )
            .map_err(|e| CorivoError::Internal(format!("创建 session 失败: {}", e)))?;
            Ok(())
        })
        .await?;

        Ok(session)
    }

    pub async fn end(&self, id: String, status: &str) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        let status = status.to_string();
        run_blocking(self.pool.clone(), move |conn| {
            conn.execute(
                "UPDATE sessions SET ended_at = ?1, status = ?2 WHERE id = ?3",
                params![now, status, id],
            )
            .map_err(|e| CorivoError::Internal(format!("结束 session 失败: {}", e)))?;
            Ok(())
        })
        .await
    }

    pub async fn increment_screenshot_count(&self, id: String) -> Result<()> {
        run_blocking(self.pool.clone(), move |conn| {
            conn.execute(
                "UPDATE sessions SET screenshot_count = screenshot_count + 1 WHERE id = ?1",
                params![id],
            )
            .map_err(|e| CorivoError::Internal(format!("更新 session 计数失败: {}", e)))?;
            Ok(())
        })
        .await
    }

    pub async fn get(&self, id: String) -> Result<Option<Session>> {
        run_blocking(self.pool.clone(), move |conn| {
            conn.query_row(
                "SELECT id, started_at, ended_at, interval_secs, screenshot_count, status
                 FROM sessions WHERE id = ?1",
                params![id],
                row_to_session,
            )
            .map(Some)
            .or_else(|e| {
                if matches!(e, rusqlite::Error::QueryReturnedNoRows) {
                    Ok(None)
                } else {
                    Err(CorivoError::Internal(format!("查询 session 失败: {}", e)))
                }
            })
        })
        .await
    }

    pub async fn list_recent(&self, limit: i64) -> Result<Vec<Session>> {
        run_blocking(self.pool.clone(), move |conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT id, started_at, ended_at, interval_secs, screenshot_count, status
                     FROM sessions ORDER BY started_at DESC LIMIT ?1",
                )
                .map_err(|e| CorivoError::Internal(format!("prepare 失败: {}", e)))?;
            let rows = stmt
                .query_map(params![limit], row_to_session)
                .map_err(|e| CorivoError::Internal(format!("query 失败: {}", e)))?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
                .map_err(|e| CorivoError::Internal(format!("collect 失败: {}", e)))
        })
        .await
    }
}

fn row_to_session(row: &rusqlite::Row) -> rusqlite::Result<Session> {
let started_at_str: String = row.get(1)?;
let ended_at_str: Option<String> = row.get(2)?;

    Ok(Session {
        id: row.get(0)?,
        started_at: DateTime::parse_from_rfc3339(&started_at_str)
            .map_err(|_| rusqlite::Error::InvalidQuery)?
            .with_timezone(&Utc),
        ended_at: ended_at_str
            .and_then(|s| DateTime::parse_from_rfc3339(&s).ok())
            .map(|dt| dt.with_timezone(&Utc)),
        interval_secs: row.get(3)?,
        screenshot_count: row.get(4)?,
        status: row.get(5)?,
    })
}

### 8.2 ScreenshotsRepo

src-tauri/src/db/repos/screenshots.rs：
rustuse crate::db::pool::{run_blocking, DbPool};
use crate::error::{CorivoError, Result};
use chrono::{DateTime, Utc};
use rusqlite::params;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Screenshot {
pub id: i64,
pub session_id: String,
pub captured_at: DateTime<Utc>,
pub file_path: String,
pub file_size: i64,
pub width: Option<i64>,
pub height: Option<i64>,
pub segment_id: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct ScreenshotInput {
pub session_id: String,
pub captured_at: DateTime<Utc>,
pub file_path: String,
pub file_size: i64,
pub width: Option<i64>,
pub height: Option<i64>,
}

#[derive(Clone)]
pub struct ScreenshotsRepo {
pool: DbPool,
}

impl ScreenshotsRepo {
pub fn new(pool: DbPool) -> Self {
Self { pool }
}

    pub async fn insert(&self, input: ScreenshotInput) -> Result<i64> {
        run_blocking(self.pool.clone(), move |conn| {
            conn.execute(
                "INSERT INTO screenshots
                 (session_id, captured_at, file_path, file_size, width, height)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    input.session_id,
                    input.captured_at.to_rfc3339(),
                    input.file_path,
                    input.file_size,
                    input.width,
                    input.height,
                ],
            )
            .map_err(|e| CorivoError::Internal(format!("插入 screenshot 失败: {}", e)))?;
            Ok(conn.last_insert_rowid())
        })
        .await
    }

    pub async fn list_by_session(&self, session_id: String) -> Result<Vec<Screenshot>> {
        run_blocking(self.pool.clone(), move |conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT id, session_id, captured_at, file_path, file_size, width, height, segment_id
                     FROM screenshots WHERE session_id = ?1 ORDER BY captured_at ASC",
                )
                .map_err(|e| CorivoError::Internal(format!("prepare 失败: {}", e)))?;
            let rows = stmt
                .query_map(params![session_id], row_to_screenshot)
                .map_err(|e| CorivoError::Internal(format!("query 失败: {}", e)))?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
                .map_err(|e| CorivoError::Internal(format!("collect 失败: {}", e)))
        })
        .await
    }

    pub async fn list_by_segment(&self, segment_id: i64) -> Result<Vec<Screenshot>> {
        run_blocking(self.pool.clone(), move |conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT id, session_id, captured_at, file_path, file_size, width, height, segment_id
                     FROM screenshots WHERE segment_id = ?1 ORDER BY captured_at ASC",
                )
                .map_err(|e| CorivoError::Internal(format!("prepare 失败: {}", e)))?;
            let rows = stmt
                .query_map(params![segment_id], row_to_screenshot)
                .map_err(|e| CorivoError::Internal(format!("query 失败: {}", e)))?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
                .map_err(|e| CorivoError::Internal(format!("collect 失败: {}", e)))
        })
        .await
    }

    /// 找出指定时间范围内还未被划入 segment 的截图（给 SegmentService 用）
    pub async fn find_unassigned(
        &self,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> Result<Vec<Screenshot>> {
        run_blocking(self.pool.clone(), move |conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT id, session_id, captured_at, file_path, file_size, width, height, segment_id
                     FROM screenshots
                     WHERE segment_id IS NULL
                       AND captured_at >= ?1 AND captured_at < ?2
                     ORDER BY captured_at ASC",
                )
                .map_err(|e| CorivoError::Internal(format!("prepare 失败: {}", e)))?;
            let rows = stmt
                .query_map(
                    params![from.to_rfc3339(), to.to_rfc3339()],
                    row_to_screenshot,
                )
                .map_err(|e| CorivoError::Internal(format!("query 失败: {}", e)))?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
                .map_err(|e| CorivoError::Internal(format!("collect 失败: {}", e)))
        })
        .await
    }

    pub async fn assign_to_segment(
        &self,
        screenshot_ids: Vec<i64>,
        segment_id: i64,
    ) -> Result<()> {
        run_blocking(self.pool.clone(), move |conn| {
            let tx = conn
                .unchecked_transaction()
                .map_err(|e| CorivoError::Internal(format!("开启事务失败: {}", e)))?;
            for sid in screenshot_ids {
                tx.execute(
                    "UPDATE screenshots SET segment_id = ?1 WHERE id = ?2",
                    params![segment_id, sid],
                )
                .map_err(|e| CorivoError::Internal(format!("更新失败: {}", e)))?;
            }
            tx.commit()
                .map_err(|e| CorivoError::Internal(format!("提交事务失败: {}", e)))?;
            Ok(())
        })
        .await
    }

    pub async fn delete_older_than(&self, cutoff: DateTime<Utc>) -> Result<i64> {
        run_blocking(self.pool.clone(), move |conn| {
            let affected = conn
                .execute(
                    "DELETE FROM screenshots WHERE captured_at < ?1",
                    params![cutoff.to_rfc3339()],
                )
                .map_err(|e| CorivoError::Internal(format!("删除失败: {}", e)))?;
            Ok(affected as i64)
        })
        .await
    }

    pub async fn count(&self) -> Result<i64> {
        run_blocking(self.pool.clone(), move |conn| {
            conn.query_row("SELECT COUNT(*) FROM screenshots", [], |r| r.get(0))
                .map_err(|e| CorivoError::Internal(format!("计数失败: {}", e)))
        })
        .await
    }
}

fn row_to_screenshot(row: &rusqlite::Row) -> rusqlite::Result<Screenshot> {
let captured_at_str: String = row.get(2)?;
Ok(Screenshot {
id: row.get(0)?,
session_id: row.get(1)?,
captured_at: DateTime::parse_from_rfc3339(&captured_at_str)
.map_err(|_| rusqlite::Error::InvalidQuery)?
.with_timezone(&Utc),
file_path: row.get(3)?,
file_size: row.get(4)?,
width: row.get(5)?,
height: row.get(6)?,
segment_id: row.get(7)?,
})
}

### 8.3 SegmentsRepo

src-tauri/src/db/repos/segments.rs：
rustuse crate::db::pool::{run_blocking, DbPool};
use crate::error::{CorivoError, Result};
use chrono::{DateTime, Utc};
use rusqlite::params;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum SegmentStatus {
Pending,
Processing,
Done,
PartialDone,
Failed,
}

impl SegmentStatus {
pub fn as_str(&self) -> &'static str {
match self {
SegmentStatus::Pending => "pending",
SegmentStatus::Processing => "processing",
SegmentStatus::Done => "done",
SegmentStatus::PartialDone => "partial_done",
SegmentStatus::Failed => "failed",
}
}

    pub fn from_str(s: &str) -> Self {
        match s {
            "pending" => SegmentStatus::Pending,
            "processing" => SegmentStatus::Processing,
            "done" => SegmentStatus::Done,
            "partial_done" => SegmentStatus::PartialDone,
            "failed" => SegmentStatus::Failed,
            _ => SegmentStatus::Failed,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Segment {
pub id: i64,
pub session_id: Option<String>,
pub started_at: DateTime<Utc>,
pub ended_at: DateTime<Utc>,
pub status: SegmentStatus,
pub prompt_used: Option<String>,
pub summary: Option<String>,
pub activity_type: Option<String>,
pub model: Option<String>,
pub input_tokens: Option<i64>,
pub output_tokens: Option<i64>,
pub cost_usd: Option<f64>,
pub error_message: Option<String>,
pub generated_at: Option<DateTime<Utc>>,
pub memory_id: Option<String>,
pub screenshot_count: i64,
pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct SegmentInput {
pub session_id: Option<String>,
pub started_at: DateTime<Utc>,
pub ended_at: DateTime<Utc>,
pub screenshot_count: i64,
}

#[derive(Debug, Clone)]
pub struct SegmentUpdate {
pub status: Option<SegmentStatus>,
pub prompt_used: Option<String>,
pub summary: Option<String>,
pub model: Option<String>,
pub input_tokens: Option<i64>,
pub output_tokens: Option<i64>,
pub cost_usd: Option<f64>,
pub error_message: Option<String>,
pub generated_at: Option<DateTime<Utc>>,
pub memory_id: Option<String>,
}

#[derive(Clone)]
pub struct SegmentsRepo {
pool: DbPool,
}

impl SegmentsRepo {
pub fn new(pool: DbPool) -> Self {
Self { pool }
}

    pub async fn create(&self, input: SegmentInput) -> Result<i64> {
        run_blocking(self.pool.clone(), move |conn| {
            conn.execute(
                "INSERT INTO segments
                 (session_id, started_at, ended_at, status, screenshot_count, created_at)
                 VALUES (?1, ?2, ?3, 'pending', ?4, ?5)",
                params![
                    input.session_id,
                    input.started_at.to_rfc3339(),
                    input.ended_at.to_rfc3339(),
                    input.screenshot_count,
                    Utc::now().to_rfc3339(),
                ],
            )
            .map_err(|e| CorivoError::Internal(format!("创建 segment 失败: {}", e)))?;
            Ok(conn.last_insert_rowid())
        })
        .await
    }

    pub async fn update(&self, id: i64, update: SegmentUpdate) -> Result<()> {
        run_blocking(self.pool.clone(), move |conn| {
            // 用一个大 UPDATE 语句，COALESCE 保留原值
            conn.execute(
                "UPDATE segments SET
                    status = COALESCE(?1, status),
                    prompt_used = COALESCE(?2, prompt_used),
                    summary = COALESCE(?3, summary),
                    model = COALESCE(?4, model),
                    input_tokens = COALESCE(?5, input_tokens),
                    output_tokens = COALESCE(?6, output_tokens),
                    cost_usd = COALESCE(?7, cost_usd),
                    error_message = COALESCE(?8, error_message),
                    generated_at = COALESCE(?9, generated_at),
                    memory_id = COALESCE(?10, memory_id)
                 WHERE id = ?11",
                params![
                    update.status.as_ref().map(|s| s.as_str()),
                    update.prompt_used,
                    update.summary,
                    update.model,
                    update.input_tokens,
                    update.output_tokens,
                    update.cost_usd,
                    update.error_message,
                    update.generated_at.map(|dt| dt.to_rfc3339()),
                    update.memory_id,
                    id,
                ],
            )
            .map_err(|e| CorivoError::Internal(format!("更新 segment 失败: {}", e)))?;
            Ok(())
        })
        .await
    }

    pub async fn get(&self, id: i64) -> Result<Option<Segment>> {
        run_blocking(self.pool.clone(), move |conn| {
            conn.query_row(
                "SELECT id, session_id, started_at, ended_at, status, prompt_used, summary,
                        activity_type, model, input_tokens, output_tokens, cost_usd,
                        error_message, generated_at, memory_id, screenshot_count, created_at
                 FROM segments WHERE id = ?1",
                params![id],
                row_to_segment,
            )
            .map(Some)
            .or_else(|e| {
                if matches!(e, rusqlite::Error::QueryReturnedNoRows) {
                    Ok(None)
                } else {
                    Err(CorivoError::Internal(format!("查询 segment 失败: {}", e)))
                }
            })
        })
        .await
    }

    /// 按日期列出 segment，用于概览页时间线
    pub async fn list_by_date(&self, date_from: DateTime<Utc>, date_to: DateTime<Utc>) -> Result<Vec<Segment>> {
        run_blocking(self.pool.clone(), move |conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT id, session_id, started_at, ended_at, status, prompt_used, summary,
                            activity_type, model, input_tokens, output_tokens, cost_usd,
                            error_message, generated_at, memory_id, screenshot_count, created_at
                     FROM segments
                     WHERE started_at >= ?1 AND started_at < ?2
                     ORDER BY started_at DESC",
                )
                .map_err(|e| CorivoError::Internal(format!("prepare 失败: {}", e)))?;
            let rows = stmt
                .query_map(
                    params![date_from.to_rfc3339(), date_to.to_rfc3339()],
                    row_to_segment,
                )
                .map_err(|e| CorivoError::Internal(format!("query 失败: {}", e)))?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
                .map_err(|e| CorivoError::Internal(format!("collect 失败: {}", e)))
        })
        .await
    }

    /// 找出所有 pending 状态的 segment（给 SummaryWorker 轮询用）
    pub async fn list_pending(&self, limit: i64) -> Result<Vec<Segment>> {
        run_blocking(self.pool.clone(), move |conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT id, session_id, started_at, ended_at, status, prompt_used, summary,
                            activity_type, model, input_tokens, output_tokens, cost_usd,
                            error_message, generated_at, memory_id, screenshot_count, created_at
                     FROM segments WHERE status = 'pending' ORDER BY started_at ASC LIMIT ?1",
                )
                .map_err(|e| CorivoError::Internal(format!("prepare 失败: {}", e)))?;
            let rows = stmt
                .query_map(params![limit], row_to_segment)
                .map_err(|e| CorivoError::Internal(format!("query 失败: {}", e)))?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
                .map_err(|e| CorivoError::Internal(format!("collect 失败: {}", e)))
        })
        .await
    }

    /// 原子地把一个 segment 从 pending 改成 processing（避免并发 worker 重复处理）
    pub async fn try_claim_pending(&self, id: i64) -> Result<bool> {
        run_blocking(self.pool.clone(), move |conn| {
            let affected = conn
                .execute(
                    "UPDATE segments SET status = 'processing'
                     WHERE id = ?1 AND status = 'pending'",
                    params![id],
                )
                .map_err(|e| CorivoError::Internal(format!("claim 失败: {}", e)))?;
            Ok(affected > 0)
        })
        .await
    }

    pub async fn latest_ended_at(&self) -> Result<Option<DateTime<Utc>>> {
        run_blocking(self.pool.clone(), move |conn| {
            let s: Option<String> = conn
                .query_row(
                    "SELECT MAX(ended_at) FROM segments",
                    [],
                    |row| row.get(0),
                )
                .map_err(|e| CorivoError::Internal(format!("查询 latest 失败: {}", e)))?;
            Ok(s.and_then(|str| DateTime::parse_from_rfc3339(&str).ok())
                .map(|dt| dt.with_timezone(&Utc)))
        })
        .await
    }
}

fn row_to_segment(row: &rusqlite::Row) -> rusqlite::Result<Segment> {
let started_at_str: String = row.get(2)?;
let ended_at_str: String = row.get(3)?;
let generated_at_str: Option<String> = row.get(13)?;
let created_at_str: String = row.get(16)?;
let status_str: String = row.get(4)?;

    Ok(Segment {
        id: row.get(0)?,
        session_id: row.get(1)?,
        started_at: DateTime::parse_from_rfc3339(&started_at_str)
            .map_err(|_| rusqlite::Error::InvalidQuery)?
            .with_timezone(&Utc),
        ended_at: DateTime::parse_from_rfc3339(&ended_at_str)
            .map_err(|_| rusqlite::Error::InvalidQuery)?
            .with_timezone(&Utc),
        status: SegmentStatus::from_str(&status_str),
        prompt_used: row.get(5)?,
        summary: row.get(6)?,
        activity_type: row.get(7)?,
        model: row.get(8)?,
        input_tokens: row.get(9)?,
        output_tokens: row.get(10)?,
        cost_usd: row.get(11)?,
        error_message: row.get(12)?,
        generated_at: generated_at_str
            .and_then(|s| DateTime::parse_from_rfc3339(&s).ok())
            .map(|dt| dt.with_timezone(&Utc)),
        memory_id: row.get(14)?,
        screenshot_count: row.get(15)?,
        created_at: DateTime::parse_from_rfc3339(&created_at_str)
            .map_err(|_| rusqlite::Error::InvalidQuery)?
            .with_timezone(&Utc),
    })
}

### 8.4 repos/mod.rs

rustpub mod screenshots;
pub mod sessions;
pub mod segments;

// P1 预留，P0 不实现
// pub mod extracted_facts;
// pub mod conflict_alerts;

## 九、Tauri Commands

src-tauri/src/commands/db_debug.rs：
rustuse crate::db::{Database, DbStats};
use std::sync::Arc;
use tauri::State;

pub struct DbAppState {
pub db: Arc<Database>,
}

#[tauri::command]
pub async fn get_db_stats(state: State<'_, DbAppState>) -> Result<DbStats, String> {
state.db.get_stats().await.map_err(Into::into)
}

#[tauri::command]
pub async fn open_data_directory(state: State<'_, DbAppState>) -> Result<(), String> {
let path = state.db.db_path().parent()
.ok_or("无法获取数据目录路径")?
.to_path_buf();

    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(&path)
            .spawn()
            .map_err(|e| format!("打开目录失败: {}", e))?;
    }
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("explorer")
            .arg(&path)
            .spawn()
            .map_err(|e| format!("打开目录失败: {}", e))?;
    }
    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("xdg-open")
            .arg(&path)
            .spawn()
            .map_err(|e| format!("打开目录失败: {}", e))?;
    }
    Ok(())
}

## 十、lib.rs 装配

rust// 在 setup 里新增

use db::Database;
use commands::db_debug::DbAppState;

let data_dir = app.path()
.app_data_dir()
.map_err(|e| format!("获取 app data dir 失败: {}", e))?;

let db = Arc::new(Database::initialize(data_dir)?);
app.manage(DbAppState { db: db.clone() });

// invoke_handler 增加:
commands::db_debug::get_db_stats,
commands::db_debug::open_data_directory,

// setup 返回 Ok 之前，注册关闭钩子
let db_for_shutdown = db.clone();
app.on_window_event(move |_, event| {
if let tauri::WindowEvent::CloseRequested { .. } = event {
if let Err(e) = db_for_shutdown.shutdown() {
tracing::error!("DB shutdown failed: {:?}", e);
}
}
});

## 十一、前端

src/lib/types.ts 追加：
tsexport interface DbStats {
file_size_bytes: number
screenshots_count: number
sessions_count: number
segments_count: number
}
src/lib/tauri.ts 追加：
tsimport type { DbStats } from './types'

export async function getDbStats(): Promise<DbStats> {
return invoke<DbStats>('get_db_stats')
}

export async function openDataDirectory(): Promise<void> {
return invoke<void>('open_data_directory')
}

## 十二、测试

关键：所有 repo 单元测试用临时目录的 SQLite 实例，跑完自动清理。
src-tauri/src/db/repos/sessions.rs 末尾：
rust#[cfg(test)]
mod tests {
use super::*;
use crate::db::Database;
use tempfile::TempDir;

    async fn setup() -> (Database, TempDir) {
        let tmp = TempDir::new().unwrap();
        let db = Database::initialize(tmp.path().to_path_buf()).unwrap();
        (db, tmp)
    }

    #[tokio::test]
    async fn test_create_and_get() {
        let (db, _tmp) = setup().await;
        let repo = db.sessions();
        let session = repo.create("s1".to_string(), 30).await.unwrap();
        assert_eq!(session.id, "s1");
        assert_eq!(session.status, "active");

        let loaded = repo.get("s1".to_string()).await.unwrap();
        assert!(loaded.is_some());
    }

    #[tokio::test]
    async fn test_end_session() {
        let (db, _tmp) = setup().await;
        let repo = db.sessions();
        repo.create("s1".to_string(), 30).await.unwrap();
        repo.end("s1".to_string(), "completed").await.unwrap();

        let loaded = repo.get("s1".to_string()).await.unwrap().unwrap();
        assert_eq!(loaded.status, "completed");
        assert!(loaded.ended_at.is_some());
    }

    #[tokio::test]
    async fn test_increment_screenshot_count() {
        let (db, _tmp) = setup().await;
        let repo = db.sessions();
        repo.create("s1".to_string(), 30).await.unwrap();
        repo.increment_screenshot_count("s1".to_string()).await.unwrap();
        repo.increment_screenshot_count("s1".to_string()).await.unwrap();

        let loaded = repo.get("s1".to_string()).await.unwrap().unwrap();
        assert_eq!(loaded.screenshot_count, 2);
    }
}
类似的测试给 screenshots 和 segments 各写一份（测 insert、list_by_session、list_by_segment、find_unassigned、assign_to_segment；segments 测 create、update、try_claim_pending、list_pending、list_by_date）。
新增 dev dependency：
toml[dev-dependencies]
tempfile = "3"

## 十三、任务分解

会话范围完成判定1依赖添加 + schema.sql + pool.rs + migrations.rscargo build 通过2db/mod.rs 顶层结构 + Database::initialize手动测试：启动 app 后 data 目录有 corivo.sqlite3SessionsRepo + 单元测试cargo test sessions::tests 通过4ScreenshotsRepo + 单元测试cargo test screenshots::tests 通过5SegmentsRepo + 单元测试cargo test segments::tests 通过6commands/db_debug.rs + lib.rs 装配 + 关闭钩子启动后能 invoke get_db_stats7前端 tauri.ts 封装 + 类型TS 类型提示正常

## 十四、验收清单

cargo test 全部通过
cargo clippy 无 warning
启动 app，data 目录下看到 corivo.sqlite、corivo.sqlite-wal、corivo.sqlite-shm
用 sqlite3 CLI 打开库能看到 5 张表 + schema_version 表
schema_version 表有 version=1 的记录
调用 get_db_stats 返回正确的行数
关闭 app 后 .sqlite-wal 文件被清空（checkpoint 生效）
重启 app 能正常打开已有数据库，不会重复建表
前端调用 openDataDirectory 能打开文件管理器到正确位置
并发测试：同时从 5 个 tokio task 向 screenshots 表插数据，无报错

## 十五、坑点预警

chrono feature 的坑：rusqlite 的 chrono feature 让 DateTime<Utc> 能直接 FromSql/ToSql，但它存的是 SQLite 原生 TIMESTAMP 格式（不是 RFC3339）。本 spec 强制用 RFC3339 字符串自己转换，避免格式歧义。
WAL 文件的清理：正常关闭 app 时 wal_checkpoint(TRUNCATE) 会把 WAL 清空。如果 app 崩溃，WAL 文件会变得很大（可能几百 MB）。SQLite 下次打开时会自动恢复，但要等一下。
unchecked_transaction：rusqlite 的事务 API 有点坑，unchecked_transaction 是避免 lifetime 问题的常用做法。别用 conn.transaction()，它要求 &mut Connection，和连接池一起用会报错。
外键 CASCADE：sessions 删了会级联删 screenshots，但 segments.session_id 是 SET NULL。这是故意的——删 session 不应该丢失已经生成好的总结。
AUTOINCREMENT vs 普通 INTEGER PRIMARY KEY：SQLite 的 INTEGER PRIMARY KEY 已经是自增的，AUTOINCREMENT 会加一个约束保证 ID 绝不重用（删掉的 ID 不会被新行用）。我们用 AUTOINCREMENT 是为了让 memory_id 关联更稳定。
连接池大小：max_size(8) 够用。WAL 模式下读并发不受限，写是串行的，8 个连接已经远超需要。
chrono RFC3339 vs ISO8601：DateTime::parse_from_rfc3339 和 to_rfc3339 是配对的，往返无损。如果有人手动塞了 2026-04-10 10:00:00 这种格式会解析失败——只能约束所有写入都走 repo 层。
r2d2_sqlite 的 version：必须和 rusqlite 的大版本对齐。rusqlite 0.31 对应 r2d2_sqlite 0.24，版本错了会编译失败。

## 十六、产出物

完成 spec-05 后你应该有：

一套完整的 SQLite 数据层，支持未来 schema 演进
三张核心表 + 两张 P1 预留表全部建好
三个功能完整、测试覆盖的 repo
一个 debug command 能实时看数据库状态
应用关闭时正确 checkpoint，不留 WAL 垃圾
后续 spec 可以直接 db.sessions() / db.screenshots() / db.segments() 拿到 repo
