use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use rand::Rng;
use serde::{Deserialize, Serialize};
use tokio::{fs, sync::RwLock};

use crate::error::{CorivoError, Result};

const SESSIONS_INDEX_FILE: &str = "sessions.json";
const SESSION_META_FILE: &str = "meta.json";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SessionStatus {
    Active,
    Completed,
    Aborted,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionInfo {
    pub id: String,
    pub started_at: DateTime<Utc>,
    pub ended_at: Option<DateTime<Utc>>,
    /// `serde(default)` so a `sessions.json` written by an older build
    /// that didn't yet have this field (or by the brief "no periodic
    /// screenshot" build that never set it) still deserialises.
    #[serde(default)]
    pub screenshot_count: i64,
    pub status: SessionStatus,
    pub interval_secs: u64,
    pub batch_size: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScreenshotFile {
    pub filename: String,
    pub sequence: i64,
    pub absolute_path: String,
    pub size_bytes: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageStats {
    pub captures_dir: String,
    pub total_size_bytes: i64,
    pub session_count: i64,
    pub screenshot_count: i64,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct SessionsIndex {
    sessions: Vec<SessionInfo>,
}

pub struct CaptureStore {
    captures_root: PathBuf,
    index: RwLock<SessionsIndex>,
}

impl CaptureStore {
    pub async fn new(app_data_dir: PathBuf) -> Result<Self> {
        let captures_root = app_data_dir.join("captures");
        fs::create_dir_all(&captures_root)
            .await
            .map_err(|error| CorivoError::Internal(format!("创建 captures 目录失败: {error}")))?;

        let mut index = Self::load_index(&captures_root).await?;

        let mut stale_fixed = false;
        for session in &mut index.sessions {
            if session.status == SessionStatus::Active {
                session.status = SessionStatus::Aborted;
                session.ended_at = Some(Utc::now());
                stale_fixed = true;
            }
        }

        let store = Self {
            captures_root,
            index: RwLock::new(index),
        };

        if stale_fixed {
            tracing::info!("fixed stale active sessions left from previous run");
            store.save_index().await?;
        }

        Ok(store)
    }

    pub async fn create_session(
        &self,
        interval_secs: u64,
        batch_size: usize,
    ) -> Result<SessionInfo> {
        let now = Utc::now();
        let session_id = Self::generate_session_id(now);
        let session_dir = self.session_dir(&session_id);
        fs::create_dir_all(&session_dir)
            .await
            .map_err(|error| CorivoError::Internal(format!("创建 session 目录失败: {error}")))?;

        let info = SessionInfo {
            id: session_id,
            started_at: now,
            ended_at: None,
            screenshot_count: 0,
            status: SessionStatus::Active,
            interval_secs,
            batch_size,
        };

        self.save_session_meta(&info).await?;
        {
            let mut index = self.index.write().await;
            index.sessions.push(info.clone());
        }
        self.save_index().await?;

        Ok(info)
    }

    pub async fn save_screenshot(
        &self,
        session_id: &str,
        jpeg_data: Vec<u8>,
    ) -> Result<(PathBuf, usize)> {
        let next_sequence = {
            let mut index = self.index.write().await;
            let session = index
                .sessions
                .iter_mut()
                .find(|session| session.id == session_id)
                .ok_or_else(|| CorivoError::Internal(format!("session {session_id} 不存在")))?;
            session.screenshot_count += 1;
            session.screenshot_count
        };

        let file_path = self
            .session_dir(session_id)
            .join(format!("{next_sequence:04}.jpg"));
        let data_len = jpeg_data.len();
        fs::write(&file_path, jpeg_data)
            .await
            .map_err(|error| CorivoError::Internal(format!("写入截图失败: {error}")))?;

        if next_sequence % 5 == 0 {
            self.save_index().await?;
            if let Some(info) = self.get_session(session_id).await? {
                self.save_session_meta(&info).await?;
            }
        }

        Ok((file_path, data_len))
    }

    pub async fn end_session(&self, session_id: &str, aborted: bool) -> Result<()> {
        let mut info_to_save = None;
        {
            let mut index = self.index.write().await;
            if let Some(session) = index
                .sessions
                .iter_mut()
                .find(|session| session.id == session_id)
            {
                session.ended_at = Some(Utc::now());
                session.status = if aborted {
                    SessionStatus::Aborted
                } else {
                    SessionStatus::Completed
                };
                info_to_save = Some(session.clone());
            }
        }

        self.save_index().await?;
        if let Some(info) = info_to_save {
            self.save_session_meta(&info).await?;
        }
        Ok(())
    }

    pub async fn get_session(&self, session_id: &str) -> Result<Option<SessionInfo>> {
        let index = self.index.read().await;
        Ok(index
            .sessions
            .iter()
            .find(|session| session.id == session_id)
            .cloned())
    }

    pub async fn list_sessions(&self) -> Result<Vec<SessionInfo>> {
        let index = self.index.read().await;
        let mut sessions = index.sessions.clone();
        sessions.sort_by(|left, right| right.started_at.cmp(&left.started_at));
        Ok(sessions)
    }

    pub async fn list_screenshots(&self, session_id: &str) -> Result<Vec<ScreenshotFile>> {
        let session_dir = self.session_dir(session_id);
        if !session_dir.exists() {
            return Ok(Vec::new());
        }

        let mut entries = fs::read_dir(&session_dir)
            .await
            .map_err(|error| CorivoError::Internal(format!("读取 session 目录失败: {error}")))?;
        let mut screenshots = Vec::new();

        while let Some(entry) = entries
            .next_entry()
            .await
            .map_err(|error| CorivoError::Internal(format!("遍历目录失败: {error}")))?
        {
            let path = entry.path();
            if path.extension().and_then(|value| value.to_str()) != Some("jpg") {
                continue;
            }

            let filename = path
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or_default()
                .to_string();
            let sequence = path
                .file_stem()
                .and_then(|value| value.to_str())
                .and_then(|value| value.parse::<i64>().ok())
                .unwrap_or(0);
            let size_bytes = entry
                .metadata()
                .await
                .map(|metadata| metadata.len() as i64)
                .unwrap_or(0);

            screenshots.push(ScreenshotFile {
                filename,
                sequence,
                absolute_path: path.to_string_lossy().to_string(),
                size_bytes,
            });
        }

        screenshots.sort_by_key(|shot| shot.sequence);
        Ok(screenshots)
    }

    pub async fn delete_session(&self, session_id: &str) -> Result<()> {
        let session_dir = self.session_dir(session_id);
        if session_dir.exists() {
            fs::remove_dir_all(&session_dir).await.map_err(|error| {
                CorivoError::Internal(format!("删除 session 目录失败: {error}"))
            })?;
        }

        {
            let mut index = self.index.write().await;
            index.sessions.retain(|session| session.id != session_id);
        }
        self.save_index().await?;
        Ok(())
    }

    pub async fn delete_older_than(&self, days: u32) -> Result<i64> {
        let cutoff = Utc::now() - chrono::Duration::days(days as i64);
        let deletable_ids = {
            let index = self.index.read().await;
            index
                .sessions
                .iter()
                .filter(|session| {
                    session.started_at < cutoff && session.status != SessionStatus::Active
                })
                .map(|session| session.id.clone())
                .collect::<Vec<_>>()
        };

        let deleted = deletable_ids.len() as i64;
        for session_id in deletable_ids {
            self.delete_session(&session_id).await?;
        }
        Ok(deleted)
    }

    pub async fn get_storage_stats(&self) -> Result<StorageStats> {
        let index = self.index.read().await;
        let session_count = index.sessions.len() as i64;
        let screenshot_count = index
            .sessions
            .iter()
            .map(|session| session.screenshot_count)
            .sum();
        drop(index);

        let total_size_bytes = calculate_dir_size(&self.captures_root).await?;

        Ok(StorageStats {
            captures_dir: self.captures_root.to_string_lossy().to_string(),
            total_size_bytes,
            session_count,
            screenshot_count,
        })
    }

    pub async fn shutdown(&self) -> Result<()> {
        self.save_index().await
    }

    pub fn captures_root(&self) -> &PathBuf {
        &self.captures_root
    }

    pub fn session_dir(&self, session_id: &str) -> PathBuf {
        self.captures_root.join(session_id)
    }

    async fn load_index(captures_root: &Path) -> Result<SessionsIndex> {
        let index_path = captures_root.join(SESSIONS_INDEX_FILE);
        match fs::read_to_string(index_path).await {
            Ok(content) => serde_json::from_str(&content).map_err(|error| {
                CorivoError::Internal(format!("解析 sessions.json 失败: {error}"))
            }),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                Ok(SessionsIndex::default())
            }
            Err(error) => Err(CorivoError::Internal(format!(
                "读取 sessions.json 失败: {error}"
            ))),
        }
    }

    async fn save_index(&self) -> Result<()> {
        let index_path = self.captures_root.join(SESSIONS_INDEX_FILE);
        let index = self.index.read().await;
        let content = serde_json::to_string_pretty(&*index).map_err(|error| {
            CorivoError::Internal(format!("序列化 sessions index 失败: {error}"))
        })?;
        fs::write(index_path, content)
            .await
            .map_err(|error| CorivoError::Internal(format!("写入 sessions.json 失败: {error}")))?;
        Ok(())
    }

    async fn save_session_meta(&self, info: &SessionInfo) -> Result<()> {
        let meta_path = self.session_dir(&info.id).join(SESSION_META_FILE);
        let content = serde_json::to_string_pretty(info)
            .map_err(|error| CorivoError::Internal(format!("序列化 meta 失败: {error}")))?;
        fs::write(meta_path, content)
            .await
            .map_err(|error| CorivoError::Internal(format!("写入 meta.json 失败: {error}")))?;
        Ok(())
    }

    fn generate_session_id(now: DateTime<Utc>) -> String {
        let mut rng = rand::thread_rng();
        let suffix = (0..4)
            .map(|_| format!("{:x}", rng.gen_range(0..16)))
            .collect::<String>();
        format!("{}-{suffix}", now.format("%Y%m%d-%H%M%S"))
    }
}

async fn calculate_dir_size(path: &Path) -> Result<i64> {
    let mut total = 0;
    let mut stack = vec![path.to_path_buf()];

    while let Some(current) = stack.pop() {
        let mut entries = match fs::read_dir(current).await {
            Ok(entries) => entries,
            Err(_) => continue,
        };

        while let Some(entry) = entries.next_entry().await.ok().flatten() {
            let metadata = match entry.metadata().await {
                Ok(metadata) => metadata,
                Err(_) => continue,
            };

            if metadata.is_dir() {
                stack.push(entry.path());
            } else {
                total += metadata.len() as i64;
            }
        }
    }

    Ok(total)
}
