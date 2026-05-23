# spec-06-screenshot-pipeline.md

## 一、目标

把 MVP 的"内存 batch + 调用链串联"升级为一个结构清晰、职责分明的截图采集流水线。完成本 spec 后：

原始截图按 session 组织存入文件系统
CaptureLoop 负责截图 + 存盘 + 触发 pipeline
MvpPipeline 负责 Gemini 总结 → 写 Supermemory → 搜相关 → 判断推送
session 的生命周期清晰（开始 / 运行中 / 结束）
用户可以浏览历史 session 和其中的截图（调试用）
整个流水线不依赖 SQLite，只依赖文件系统 + Supermemory
和 MVP 的区别：MVP 里所有截图只存内存的 Vec<ImageInput>，处理完就丢。本 spec 让截图落地到磁盘并可回看，同时把流水线拆成可测试的独立模块。

## 二、不做什么

- ❌ 不引入 SQLite（上一轮讨论已砍）
- ❌ 不做 segment 概念（每一批次处理就是一个"批次"，不建立持久实体）
- ❌ 不做 pHash 去重（P1 性能优化）
- ❌ 不做空闲检测 / 用户离开暂停（P1）
- ❌ 不做多显示器支持（只截主屏）
- ❌ 不做快流水线 / 结构化抽取（P1）
- ❌ 不做事件总线广播（P1 加 SignalService 时再引入）
- ❌ 不做"重新生成某批次的总结"（需要有持久化概念，spec 后面再加）

## 三、成功标准

启动 app，点连接页的「开始捕获」，后台每 15 秒截一张屏幕
app data 目录下出现 captures/<session_id>/0001.jpg, 0002.jpg, ...
每攒够 N 张（默认 5 张），pipeline 自动执行：Gemini 总结 → 写 Supermemory → 搜相关 → 判断推送
点「停止捕获」，当前 session 被标记为结束，新的批次不再处理
重新点「开始捕获」，创建一个新的 session，不污染上一个
连接页能列出历史 session 及其截图数量
点击某个历史 session 能看到该 session 的所有截图（至少能看到文件列表）
配置页能配置截图间隔和批次大小，改完立即生效
整个过程无 panic、无内存泄漏、CPU 占用合理

## 四、架构

┌────────────────────────────────────────────────────────┐

│                   Corivo App                            │
│                                                         │
│  ┌──────────────┐                                      │
│  │ CaptureStore │  ← 管理文件系统上的 sessions 和截图 │
│  └──────┬───────┘                                      │
│         │                                              │
│         │  每张截图写盘                                │
│         │                                              │
│  ┌──────▼───────┐                                      │
│  │ CaptureLoop  │  ← tokio task, 定时截图             │
│  │              │                                      │
│  │ 内存中维护:  │                                      │
│  │  - current   │                                      │
│  │    session   │                                      │
│  │  - batch     │  每 N 张触发一次 on_batch_ready     │
│  │    buffer    │                                      │
│  └──────┬───────┘                                      │
│         │                                              │
│         │  传 batch 给 pipeline                        │
│         │                                              │
│  ┌──────▼───────┐                                      │
│  │ MvpPipeline  │  ← 纯业务逻辑, 不关心截图怎么来的   │
│  │              │                                      │
│  │  1. 总结     │                                      │
│  │  2. 写记忆   │                                      │
│  │  3. 搜相关   │                                      │
│  │  4. 判推送   │                                      │
│  │  5. 发通知   │                                      │
│  └──────────────┘                                      │
└────────────────────────────────────────────────────────┘

三层职责分离：

CaptureStore：文件系统抽象层，只管 IO
CaptureLoop：时序和批次管理，只管节奏
MvpPipeline：业务逻辑，只管"一批截图进来怎么处理"
三层之间通过 trait / callback 解耦，每层都能独立测试。

## 五、文件系统布局

~/Library/Application Support/com.corivo.app/
├── captures/
│   ├── sessions.json              所有 session 的元信息
│   └── 20260410-143022-a3f2/      session_id = 日期-随机
│       ├── meta.json              这个 session 的元信息
│       ├── 0001.jpg
│       ├── 0002.jpg
│       └── ...

设计取舍：

session_id 自带时间前缀：目录按字母排序就是按时间排序，ls captures/ 就是时间线
每个 session 有独立 meta.json：保存 prompt、统计、结束原因，即使顶层 sessions.json 坏了也能恢复
顶层 sessions.json 是索引：列 session 列表时不用扫目录，但坏了也能重建
图片用 4 位数字序号命名：截图 9999 张够用（大约 2.3 天连续跑），如果到上限自动转新 session

## 六、新增依赖

src-tauri/Cargo.toml：

toml

xcap = "0.0.14"           # 如果 spec-mvp 已经加了就不用重复
image = "0.25"
tauri-plugin-notification = "2"  # mvp 已经加了
rand = "0.8"              # 生成 session_id 随机后缀

## 七、CaptureStore 实现

src-tauri/src/services/capture_store.rs：
rust
use crate::error::{CorivoError, Result};
use chrono::{DateTime, Utc};
use rand::Rng;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tokio::fs;
use tokio::sync::RwLock;

const SESSIONS_INDEX_FILE: &str = "sessions.json";
const SESSION_META_FILE: &str = "meta.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionInfo {
pub id: String,
pub started_at: DateTime<Utc>,
pub ended_at: Option<DateTime<Utc>>,
pub screenshot_count: i64,
pub status: SessionStatus,
pub interval_secs: u64,
pub batch_size: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum SessionStatus {
Active,
Completed,
Aborted,
}

#[derive(Debug, Serialize, Deserialize, Default)]
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
.map_err(|e| CorivoError::Internal(format!("创建 captures 目录失败: {}", e)))?;

        let index = Self::load_index(&captures_root).await?;
        Ok(Self {
            captures_root,
            index: RwLock::new(index),
        })
    }

    async fn load_index(captures_root: &Path) -> Result<SessionsIndex> {
        let index_path = captures_root.join(SESSIONS_INDEX_FILE);
        match fs::read_to_string(&index_path).await {
            Ok(content) => serde_json::from_str(&content)
                .map_err(|e| CorivoError::Internal(format!("解析 sessions.json 失败: {}", e))),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(SessionsIndex::default()),
            Err(e) => Err(CorivoError::Internal(format!("读取 sessions.json 失败: {}", e))),
        }
    }

    async fn save_index(&self) -> Result<()> {
        let index_path = self.captures_root.join(SESSIONS_INDEX_FILE);
        let index = self.index.read().await;
        let content = serde_json::to_string_pretty(&*index)
            .map_err(|e| CorivoError::Internal(format!("序列化 index 失败: {}", e)))?;
        fs::write(&index_path, content)
            .await
            .map_err(|e| CorivoError::Internal(format!("写入 sessions.json 失败: {}", e)))?;
        Ok(())
    }

    /// 创建新 session，返回 session_id
    pub async fn create_session(
        &self,
        interval_secs: u64,
        batch_size: usize,
    ) -> Result<SessionInfo> {
        let now = Utc::now();
        let session_id = Self::generate_session_id(now);
        let session_dir = self.captures_root.join(&session_id);
        fs::create_dir_all(&session_dir)
            .await
            .map_err(|e| CorivoError::Internal(format!("创建 session 目录失败: {}", e)))?;

        let info = SessionInfo {
            id: session_id,
            started_at: now,
            ended_at: None,
            screenshot_count: 0,
            status: SessionStatus::Active,
            interval_secs,
            batch_size,
        };

        // 写 session meta.json
        self.save_session_meta(&info).await?;

        // 更新 index
        {
            let mut index = self.index.write().await;
            index.sessions.push(info.clone());
        }
        self.save_index().await?;

        tracing::info!("created session: {}", info.id);
        Ok(info)
    }

    fn generate_session_id(now: DateTime<Utc>) -> String {
        let suffix: String = (0..4)
            .map(|_| {
                let c = rand::thread_rng().gen_range(0..16);
                std::char::from_digit(c, 16).unwrap()
            })
            .collect();
        format!("{}-{}", now.format("%Y%m%d-%H%M%S"), suffix)
    }

    async fn save_session_meta(&self, info: &SessionInfo) -> Result<()> {
        let meta_path = self.captures_root.join(&info.id).join(SESSION_META_FILE);
        let content = serde_json::to_string_pretty(info)
            .map_err(|e| CorivoError::Internal(format!("序列化 meta 失败: {}", e)))?;
        fs::write(&meta_path, content)
            .await
            .map_err(|e| CorivoError::Internal(format!("写入 meta.json 失败: {}", e)))?;
        Ok(())
    }

    /// 保存一张截图到指定 session，返回绝对路径和 JPEG 字节数
    pub async fn save_screenshot(
        &self,
        session_id: &str,
        jpeg_data: Vec<u8>,
    ) -> Result<(PathBuf, usize)> {
        // 确定文件序号
        let next_seq = {
            let mut index = self.index.write().await;
            let session = index
                .sessions
                .iter_mut()
                .find(|s| s.id == session_id)
                .ok_or_else(|| {
                    CorivoError::Internal(format!("session {} 不存在", session_id))
                })?;
            session.screenshot_count += 1;
            session.screenshot_count
        };

        let filename = format!("{:04}.jpg", next_seq);
        let file_path = self.captures_root.join(session_id).join(&filename);
        let data_len = jpeg_data.len();

        fs::write(&file_path, jpeg_data)
            .await
            .map_err(|e| CorivoError::Internal(format!("写入截图失败: {}", e)))?;

        // 定期更新 index 和 meta（每 5 张写一次，避免频繁 IO）
        if next_seq % 5 == 0 {
            if let Err(e) = self.save_index().await {
                tracing::warn!("save_index failed: {:?}", e);
            }
            if let Some(info) = self.get_session(session_id).await? {
                let _ = self.save_session_meta(&info).await;
            }
        }

        Ok((file_path, data_len))
    }

    pub async fn end_session(&self, session_id: &str, aborted: bool) -> Result<()> {
        let mut info_to_save = None;
        {
            let mut index = self.index.write().await;
            if let Some(session) = index.sessions.iter_mut().find(|s| s.id == session_id) {
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
            tracing::info!(
                "ended session: {} (status={:?}, screenshots={})",
                info.id,
                info.status,
                info.screenshot_count
            );
        }
        Ok(())
    }

    pub async fn get_session(&self, session_id: &str) -> Result<Option<SessionInfo>> {
        let index = self.index.read().await;
        Ok(index.sessions.iter().find(|s| s.id == session_id).cloned())
    }

    pub async fn list_sessions(&self) -> Result<Vec<SessionInfo>> {
        let index = self.index.read().await;
        let mut sessions = index.sessions.clone();
        sessions.sort_by(|a, b| b.started_at.cmp(&a.started_at));
        Ok(sessions)
    }

    pub async fn list_screenshots(&self, session_id: &str) -> Result<Vec<ScreenshotFile>> {
        let session_dir = self.captures_root.join(session_id);
        if !session_dir.exists() {
            return Ok(Vec::new());
        }

        let mut entries = fs::read_dir(&session_dir)
            .await
            .map_err(|e| CorivoError::Internal(format!("读取 session 目录失败: {}", e)))?;

        let mut screenshots = Vec::new();
        while let Some(entry) = entries
            .next_entry()
            .await
            .map_err(|e| CorivoError::Internal(format!("遍历目录失败: {}", e)))?
        {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) == Some("jpg") {
                let filename = path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("")
                    .to_string();
                let size = entry
                    .metadata()
                    .await
                    .map(|m| m.len() as i64)
                    .unwrap_or(0);
                screenshots.push(ScreenshotFile {
                    filename: path
                        .file_name()
                        .and_then(|s| s.to_str())
                        .unwrap_or("")
                        .to_string(),
                    sequence: filename.parse().unwrap_or(0),
                    absolute_path: path.to_string_lossy().to_string(),
                    size_bytes: size,
                });
            }
        }

        screenshots.sort_by_key(|s| s.sequence);
        Ok(screenshots)
    }

    pub async fn delete_session(&self, session_id: &str) -> Result<()> {
        let session_dir = self.captures_root.join(session_id);
        if session_dir.exists() {
            fs::remove_dir_all(&session_dir)
                .await
                .map_err(|e| CorivoError::Internal(format!("删除 session 目录失败: {}", e)))?;
        }

        {
            let mut index = self.index.write().await;
            index.sessions.retain(|s| s.id != session_id);
        }
        self.save_index().await?;
        Ok(())
    }

    pub async fn delete_older_than(&self, days: u32) -> Result<i64> {
        let cutoff = Utc::now() - chrono::Duration::days(days as i64);
        let to_delete: Vec<String> = {
            let index = self.index.read().await;
            index
                .sessions
                .iter()
                .filter(|s| s.started_at < cutoff)
                .map(|s| s.id.clone())
                .collect()
        };

        let count = to_delete.len() as i64;
        for id in to_delete {
            if let Err(e) = self.delete_session(&id).await {
                tracing::error!("删除 session {} 失败: {:?}", id, e);
            }
        }
        Ok(count)
    }

    pub async fn get_storage_stats(&self) -> Result<StorageStats> {
        let index = self.index.read().await;
        let session_count = index.sessions.len() as i64;
        let screenshot_count: i64 = index.sessions.iter().map(|s| s.screenshot_count).sum();
        drop(index);

        // 递归计算 captures 目录大小
        let total_size = calculate_dir_size(&self.captures_root).await?;

        Ok(StorageStats {
            captures_dir: self.captures_root.to_string_lossy().to_string(),
            total_size_bytes: total_size,
            session_count,
            screenshot_count,
        })
    }

    pub fn captures_root(&self) -> &PathBuf {
        &self.captures_root
    }
}

async fn calculate_dir_size(path: &Path) -> Result<i64> {
let mut total: i64 = 0;
let mut stack = vec![path.to_path_buf()];
while let Some(p) = stack.pop() {
let mut entries = match fs::read_dir(&p).await {
Ok(e) => e,
Err(_) => continue,
};
while let Some(entry) = entries.next_entry().await.ok().flatten() {
let meta = match entry.metadata().await {
Ok(m) => m,
Err(_) => continue,
};
if meta.is_dir() {
stack.push(entry.path());
} else {
total += meta.len() as i64;
}
}
}
Ok(total)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScreenshotFile {
pub filename: String,
pub sequence: i64,
pub absolute_path: String,
pub size_bytes: i64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct StorageStats {
pub captures_dir: String,
pub total_size_bytes: i64,
pub session_count: i64,
pub screenshot_count: i64,
}关键设计点：

RwLock<SessionsIndex>：读多写少（list 频繁，create 和 end 少），RwLock 合适
每 5 张截图才 flush 一次 index：避免频繁 IO 影响性能
app 关闭时必须显式 flush：所以需要一个 shutdown 方法，lib.rs 的关闭钩子要调
calculate_dir_size 用栈迭代而不是递归：避免深层嵌套栈溢出（虽然不太可能）
补充一个 shutdown 方法到 CaptureStore：rustpub async fn shutdown(&self) -> Result<()> {
tracing::info!("CaptureStore shutting down, flushing index...");
self.save_index().await?;
Ok(())
}

## 八、CaptureLoop 重构

src-tauri/src/services/capture_loop.rs：
rust
use crate::error::{CorivoError, Result};
use crate::providers::llm::ImageInput;
use crate::services::capture_store::{CaptureStore, SessionInfo};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::time::{sleep, Duration};
use xcap::Monitor;

#[derive(Debug, Clone)]
pub struct CaptureRuntimeConfig {
pub interval_secs: u64,
pub batch_size: usize,
pub jpeg_quality: u8,
}

impl Default for CaptureRuntimeConfig {
fn default() -> Self {
Self {
interval_secs: 15,
batch_size: 5,
jpeg_quality: 75,
}
}
}

pub struct CapturedBatch {
pub session_id: String,
pub images: Vec<ImageInput>,
pub paths: Vec<PathBuf>,
}

pub struct CaptureLoop {
running: Arc<AtomicBool>,
config: Arc<Mutex<CaptureRuntimeConfig>>,
current_session_id: Arc<Mutex<Option<String>>>,
store: Arc<CaptureStore>,
}

impl CaptureLoop {
pub fn new(store: Arc<CaptureStore>, config: CaptureRuntimeConfig) -> Self {
Self {
running: Arc::new(AtomicBool::new(false)),
config: Arc::new(Mutex::new(config)),
current_session_id: Arc::new(Mutex::new(None)),
store,
}
}

    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    pub async fn current_session(&self) -> Option<String> {
        self.current_session_id.lock().await.clone()
    }

    /// 启动捕获。on_batch_ready 是 pipeline 的回调，每攒满一批触发一次。
    pub async fn start<F, Fut>(&self, on_batch_ready: F) -> Result<SessionInfo>
    where
        F: Fn(CapturedBatch) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = ()> + Send,
    {
        if self.running.swap(true, Ordering::SeqCst) {
            return Err(CorivoError::Internal("CaptureLoop 已在运行".to_string()));
        }

        let cfg = self.config.lock().await.clone();
        let session_info = self
            .store
            .create_session(cfg.interval_secs, cfg.batch_size)
            .await?;
        *self.current_session_id.lock().await = Some(session_info.id.clone());

        let running = self.running.clone();
        let config = self.config.clone();
        let current_session_id = self.current_session_id.clone();
        let store = self.store.clone();
        let callback = Arc::new(on_batch_ready);
        let session_id_for_task = session_info.id.clone();

        tokio::spawn(async move {
            let mut batch_images: Vec<ImageInput> = Vec::new();
            let mut batch_paths: Vec<PathBuf> = Vec::new();

            while running.load(Ordering::SeqCst) {
                let current_cfg = config.lock().await.clone();
                let active_session_id = current_session_id.lock().await.clone();

                let session_id = match active_session_id {
                    Some(id) => id,
                    None => {
                        tracing::warn!("no active session, stopping capture loop");
                        break;
                    }
                };

                // 1. 截图
                match Self::take_screenshot(current_cfg.jpeg_quality) {
                    Ok((jpeg_bytes, img_input)) => {
                        // 2. 存盘
                        match store.save_screenshot(&session_id, jpeg_bytes).await {
                            Ok((path, _)) => {
                                batch_images.push(img_input);
                                batch_paths.push(path);
                                tracing::debug!(
                                    "captured {}/{}",
                                    batch_images.len(),
                                    current_cfg.batch_size
                                );

                                // 3. 攒够一批则触发回调
                                if batch_images.len() >= current_cfg.batch_size {
                                    let images = std::mem::take(&mut batch_images);
                                    let paths = std::mem::take(&mut batch_paths);
                                    let batch = CapturedBatch {
                                        session_id: session_id.clone(),
                                        images,
                                        paths,
                                    };
                                    tracing::info!(
                                        "batch ready: {} images, session={}",
                                        batch.images.len(),
                                        batch.session_id
                                    );
                                    let cb = callback.clone();
                                    tokio::spawn(async move {
                                        cb(batch).await;
                                    });
                                }
                            }
                            Err(e) => {
                                tracing::error!("save screenshot failed: {:?}", e);
                            }
                        }
                    }
                    Err(e) => {
                        tracing::error!("take screenshot failed: {:?}", e);
                    }
                }

                sleep(Duration::from_secs(current_cfg.interval_secs)).await;
            }

            tracing::info!("capture loop task exited for session {}", session_id_for_task);
        });

        Ok(session_info)
    }

    pub async fn stop(&self) -> Result<()> {
        if !self.running.swap(false, Ordering::SeqCst) {
            return Ok(());
        }

        let session_id = self.current_session_id.lock().await.take();
        if let Some(id) = session_id {
            self.store.end_session(&id, false).await?;
        }
        Ok(())
    }

    pub async fn update_config(&self, new_config: CaptureRuntimeConfig) {
        *self.config.lock().await = new_config;
        tracing::info!("capture config updated");
    }

    fn take_screenshot(jpeg_quality: u8) -> Result<(Vec<u8>, ImageInput)> {
        let monitors = Monitor::all()
            .map_err(|e| CorivoError::Internal(format!("获取显示器失败: {}", e)))?;
        let monitor = monitors
            .into_iter()
            .next()
            .ok_or_else(|| CorivoError::Internal("没有可用显示器".to_string()))?;

        let capture = monitor
            .capture_image()
            .map_err(|e| CorivoError::Internal(format!("截图失败: {}", e)))?;

        let mut jpeg_buf = std::io::Cursor::new(Vec::new());
        let rgb_image = image::DynamicImage::ImageRgba8(capture).to_rgb8();
        let encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(
            &mut jpeg_buf,
            jpeg_quality,
        );
        encoder
            .encode(
                rgb_image.as_raw(),
                rgb_image.width(),
                rgb_image.height(),
                image::ExtendedColorType::Rgb8,
            )
            .map_err(|e| CorivoError::Internal(format!("JPEG 编码失败: {}", e)))?;

        let jpeg_bytes = jpeg_buf.into_inner();
        let img_input = ImageInput::jpeg(jpeg_bytes.clone());
        Ok((jpeg_bytes, img_input))
    }
}关键改进：

session 生命周期绑定到 start/stop：点开始创建 session，点停止结束 session
batch 回调用 tokio::spawn 异步执行：不阻塞截图节奏，即使 pipeline 处理慢也不影响下一张截图
JPEG 字节只编码一次：同时给文件系统和 Gemini，不重复编码

## 九、MvpPipeline 调整

src-tauri/src/services/mvp_pipeline.rs 只需把 process_batch 的签名从 Vec<ImageInput> 改为 CapturedBatch，内部逻辑基本不变（spec-mvp 里已经写好）。关键改动：
rust
use crate::services::capture_loop::CapturedBatch;

impl MvpPipeline {
pub async fn process_batch(&self, batch: CapturedBatch) {
let image_count = batch.images.len();
tracing::info!(
"pipeline processing {} images from session {}",
image_count,
batch.session_id
);

        // 后续步骤和 mvp spec 的 process_batch 完全一样：
        // 1. 调 Gemini 总结
        // 2. 写 Supermemory
        // 3. 搜相关
        // 4. 判断推送
        // 5. 发通知
        
        // metadata 里多塞一份 session_id 方便调试
        let memory_input = MemoryInput {
            content: summary.clone(),
            source: MemorySource::Screenshot {
                session_id: batch.session_id.clone(),
                segment_id: Utc::now().timestamp(),
            },
            tags: vec!["screenshot".to_string(), "auto".to_string()],
            occurred_at: Utc::now(),
            metadata: serde_json::json!({
                "image_count": image_count,
                "session_id": batch.session_id,
            }),
        };
        // ... 后续不变
    }
}

## 十、Tauri Commands

src-tauri/src/commands/capture.rs（全面重写）：
rust
use crate::services::capture_loop::{CaptureLoop, CaptureRuntimeConfig};
use crate::services::capture_store::{
CaptureStore, ScreenshotFile, SessionInfo, StorageStats,
};
use crate::services::mvp_pipeline::MvpPipeline;
use std::sync::Arc;
use tauri::State;

pub struct CaptureAppState {
pub capture_loop: Arc<CaptureLoop>,
pub capture_store: Arc<CaptureStore>,
pub pipeline: Arc<MvpPipeline>,
}

#[tauri::command]
pub async fn start_capture(
state: State<'_, CaptureAppState>,
) -> Result<SessionInfo, String> {
if state.capture_loop.is_running() {
return Err("已经在捕获中".to_string());
}
let pipeline = state.pipeline.clone();
state
.capture_loop
.start(move |batch| {
let p = pipeline.clone();
async move {
p.process_batch(batch).await;
}
})
.await
.map_err(Into::into)
}

#[tauri::command]
pub async fn stop_capture(state: State<'_, CaptureAppState>) -> Result<(), String> {
state.capture_loop.stop().await.map_err(Into::into)
}

#[tauri::command]
pub async fn get_capture_status(
state: State<'_, CaptureAppState>,
) -> Result<CaptureStatus, String> {
Ok(CaptureStatus {
running: state.capture_loop.is_running(),
current_session_id: state.capture_loop.current_session().await,
})
}

#[tauri::command]
pub async fn update_capture_config(
state: State<'_, CaptureAppState>,
interval_secs: u64,
batch_size: usize,
jpeg_quality: Option<u8>,
) -> Result<(), String> {
let cfg = CaptureRuntimeConfig {
interval_secs,
batch_size,
jpeg_quality: jpeg_quality.unwrap_or(75),
};
state.capture_loop.update_config(cfg).await;
Ok(())
}

#[tauri::command]
pub async fn list_sessions(
state: State<'_, CaptureAppState>,
) -> Result<Vec<SessionInfo>, String> {
state.capture_store.list_sessions().await.map_err(Into::into)
}

#[tauri::command]
pub async fn list_session_screenshots(
state: State<'_, CaptureAppState>,
session_id: String,
) -> Result<Vec<ScreenshotFile>, String> {
state
.capture_store
.list_screenshots(&session_id)
.await
.map_err(Into::into)
}

#[tauri::command]
pub async fn delete_session(
state: State<'_, CaptureAppState>,
session_id: String,
) -> Result<(), String> {
state
.capture_store
.delete_session(&session_id)
.await
.map_err(Into::into)
}

#[tauri::command]
pub async fn get_storage_stats(
state: State<'_, CaptureAppState>,
) -> Result<StorageStats, String> {
state
.capture_store
.get_storage_stats()
.await
.map_err(Into::into)
}

#[tauri::command]
pub async fn cleanup_old_sessions(
state: State<'_, CaptureAppState>,
days: u32,
) -> Result<i64, String> {
state
.capture_store
.delete_older_than(days)
.await
.map_err(Into::into)
}

#[derive(Debug, serde::Serialize)]
pub struct CaptureStatus {
pub running: bool,
pub current_session_id: Option<String>,
}

## 十一、lib.rs 装配

rust
// 在 setup 里

use services::capture_store::CaptureStore;
use services::capture_loop::{CaptureLoop, CaptureRuntimeConfig};
use services::mvp_pipeline::MvpPipeline;
use commands::capture::CaptureAppState;

let app_data_dir = app
.path()
.app_data_dir()
.map_err(|e| format!("获取 app data dir 失败: {}", e))?;

let capture_store = Arc::new(
tauri::async_runtime::block_on(CaptureStore::new(app_data_dir))
.map_err(|e| format!("初始化 CaptureStore 失败: {}", e))?,
);

let capture_config = CaptureRuntimeConfig {
interval_secs: config_service.get().capture.interval_secs,
batch_size: 5,  // 这个先 hardcode,等 Config 加字段再接入
jpeg_quality: 75,
};

let capture_loop = Arc::new(CaptureLoop::new(capture_store.clone(), capture_config));

let pipeline = Arc::new(MvpPipeline::new(
llm_service.clone(),
memory_service.clone(),
app.handle().clone(),
));

app.manage(CaptureAppState {
capture_loop: capture_loop.clone(),
capture_store: capture_store.clone(),
pipeline: pipeline.clone(),
});

// 注册 notification plugin（如果 mvp 已加，忽略）
// tauri_plugin_notification::init()

// invoke_handler 增加
commands::capture::start_capture,
commands::capture::stop_capture,
commands::capture::get_capture_status,
commands::capture::update_capture_config,
commands::capture::list_sessions,
commands::capture::list_session_screenshots,
commands::capture::delete_session,
commands::capture::get_storage_stats,
commands::capture::cleanup_old_sessions,

// 关闭钩子：flush index
let store_for_shutdown = capture_store.clone();
app.on_window_event(move |_, event| {
if let tauri::WindowEvent::CloseRequested { .. } = event {
let store = store_for_shutdown.clone();
tauri::async_runtime::spawn(async move {
if let Err(e) = store.shutdown().await {
tracing::error!("CaptureStore shutdown failed: {:?}", e);
}
});
}
});

## 十二、前端改动

src/lib/types.ts 追加：
ts
export type SessionStatus = 'active' | 'completed' | 'aborted'

export interface SessionInfo {
id: string
started_at: string
ended_at: string | null
screenshot_count: number
status: SessionStatus
interval_secs: number
batch_size: number
}

export interface ScreenshotFile {
filename: string
sequence: number
absolute_path: string
size_bytes: number
}

export interface CaptureStatus {
running: boolean
current_session_id: string | null
}

export interface StorageStats {
captures_dir: string
total_size_bytes: number
session_count: number
screenshot_count: number
}src/lib/tauri.ts 对 capture 相关的 command 做更新（替换 MVP 里的简化版）：tsexport async function startCapture(): Promise<SessionInfo> {
return invoke<SessionInfo>('start_capture')
}

export async function stopCapture(): Promise<void> {
return invoke<void>('stop_capture')
}

export async function getCaptureStatus(): Promise<CaptureStatus> {
return invoke<CaptureStatus>('get_capture_status')
}

export async function updateCaptureConfig(
intervalSecs: number,
batchSize: number,
jpegQuality?: number
): Promise<void> {
return invoke<void>('update_capture_config', {
intervalSecs,
batchSize,
jpegQuality: jpegQuality ?? null,
})
}

export async function listSessions(): Promise<SessionInfo[]> {
return invoke<SessionInfo[]>('list_sessions')
}

export async function listSessionScreenshots(
sessionId: string
): Promise<ScreenshotFile[]> {
return invoke<ScreenshotFile[]>('list_session_screenshots', { sessionId })
}

export async function deleteSession(sessionId: string): Promise<void> {
return invoke<void>('delete_session', { sessionId })
}

export async function getStorageStats(): Promise<StorageStats> {
return invoke<StorageStats>('get_storage_stats')
}

export async function cleanupOldSessions(days: number): Promise<number> {
return invoke<number>('cleanup_old_sessions', { days })
}

## 十三、连接页 UI 扩展

把 MVP 版的连接页扩展一下，增加「历史会话」区域：

src/pages/connections/connections-page.tsx 在现有的捕获控制卡片下面追加：
tsx
import { formatDistanceToNow, format } from 'date-fns'
import { zhCN } from 'date-fns/locale'
import { ChevronRight, Trash2 } from 'lucide-react'
import { Link } from '@tanstack/react-router'

// ... 现有代码

// 在 captureCard 下面加
const { data: sessions } = useQuery({
queryKey: ['sessions'],
queryFn: listSessions,
refetchInterval: 5000,
})

// 在 return 里加
<div>
  <div className="flex items-center justify-between mb-3">
    <h2 className="text-sm font-medium">历史会话</h2>
    <span className="text-xs text-muted-foreground">
      {sessions?.length ?? 0} 个会话
    </span>
  </div>
  <div className="space-y-2">
    {sessions?.map((s) => (
      <Link
        key={s.id}
        to="/connections/screenshot"
        search={{ session: s.id }}
        className="block"
      >
        <Card className="p-3 hover:bg-accent/40 transition-colors">
          <div className="flex items-center gap-3">
            <div className="flex-1 min-w-0">
              <div className="text-sm font-medium truncate">
                {format(new Date(s.started_at), 'MM月dd日 HH:mm', { locale: zhCN })}
              </div>
              <div className="text-xs text-muted-foreground">
                {s.screenshot_count} 张截图 ·{' '}
                {s.status === 'active'
                  ? '运行中'
                  : formatDistanceToNow(new Date(s.ended_at ?? s.started_at), {
                      addSuffix: true,
                      locale: zhCN,
                    })}
              </div>
            </div>
            <ChevronRight className="w-4 h-4 text-muted-foreground" />
          </div>
        </Card>
      </Link>
    ))}
    {sessions?.length === 0 && (
      <div className="text-xs text-muted-foreground py-6 text-center">
        还没有会话，点击上方「开始」开始第一次捕获
      </div>
    )}
  </div>
</div>截图详情页（原有的占位符替换）：src/pages/connections/screenshot-detail-page.tsx：tsximport { useQuery } from '@tanstack/react-query'
import { useSearch } from '@tanstack/react-router'
import { convertFileSrc } from '@tauri-apps/api/core'
import { listSessionScreenshots } from '@/lib/tauri'

export function ScreenshotDetailPage() {
const { session } = useSearch({ from: '/connections/screenshot' }) as {
session?: string
}

const { data: screenshots, isLoading } = useQuery({
queryKey: ['session-screenshots', session],
queryFn: () => listSessionScreenshots(session!),
enabled: !!session,
})

if (!session) {
return (
<div className="text-sm text-muted-foreground">
未指定 session
</div>
)
}

return (
<div className="space-y-4">
<div>
<h1 className="text-xl font-semibold">会话 {session}</h1>
<p className="text-xs text-muted-foreground mt-1">
{screenshots?.length ?? 0} 张截图
</p>
</div>
{isLoading && <div className="text-sm text-muted-foreground">加载中...</div>}
<div className="grid grid-cols-3 gap-3">
{screenshots?.map((s) => (
<div key={s.filename} className="rounded-md border border-border overflow-hidden">
<img
src={convertFileSrc(s.absolute_path)}
alt={s.filename}
className="w-full aspect-video object-cover"
loading="lazy"
/>
<div className="px-2 py-1 text-xs text-muted-foreground">
{s.filename} · {(s.size_bytes / 1024).toFixed(0)} KB
</div>
</div>
))}
</div>
</div>
)
}注意 screenshot route 要接受 session search param，回去改 src/routes/connections.screenshot.tsx：tsximport { z } from 'zod'
// ...

const searchSchema = z.object({
session: z.string().optional(),
})

export const Route = createRoute({
getParentRoute: () => RootRoute,
path: '/connections/screenshot',
validateSearch: searchSchema,
component: ScreenshotDetailPage,
})

## 十四、capabilities 配置

src-tauri/capabilities/default.json 的 permissions 里追加：
json
"fs:allow-app-read",
"fs:scope-app-recursive"这是为了让 convertFileSrc 能把 captures/ 目录下的截图转成前端能读的 URL。

## 十五、任务分解

会话范围完成判定
1. CaptureStore 实现 + 单元测试（用 tempfile）
2. CaptureLoop 重构（接入 CaptureStore） + 基础单元测试
3. MvpPipeline 接入 CapturedBatch + metadata 更新
4. commands/capture.rs 全部 command + lib.rs 装配（含关闭钩子）
5. 前端 types/tauri.ts 更新 + 连接页历史会话区域
6. 截图详情页 + convertFileSrc 集成 + capabilities 配置
7. 端到端手动测试 + 调 bug

## 十六、验收清单
点「开始捕获」创建新 session，data 目录出现 captures/<id>/ 目录
每 15 秒看到 captured N/M 日志，对应目录下有 JPEG 文件
每攒够 5 张触发 pipeline，看到 Gemini 总结、写 Supermemory、搜索、推送判断的全流程日志
点「停止捕获」，sessions.json 里这个 session 变成 completed 状态
重新点「开始」，创建新 session，不和旧的混在一起
连接页能看到历史 session 列表（按时间倒序）
点某个 session 进入详情页，能看到所有截图缩略图
改截图间隔（10s）和批次大小（3）后点「更新配置」立即生效
调用 get_storage_stats 返回正确的大小和计数
调用 cleanup_old_sessions(0) 删掉所有已结束的 session，目录和 index 都清理掉
app 关闭再打开，sessions.json 完整，能看到之前的 session
连续跑 1 小时，内存占用稳定（<200MB）、无 crash

## 十七、坑点预警

macOS 屏幕录制权限：第一次 xcap 截图触发系统权限弹窗，授权后必须重启 app。在连接页开始按钮旁加一句"首次使用需要授权屏幕录制权限"。

convertFileSrc 在 Tauri 2 的用法：import { convertFileSrc } from '@tauri-apps/api/core'。返回 asset://localhost/path 协议 URL，前端可以直接 <img src>。

fs:scope-app-recursive：这个权限必须加，否则 convertFileSrc 返回的 URL 虽然能生成但加载时 403。

异步锁的死锁：capture_loop.start 里持有 current_session_id 的锁时不要调用会重新加锁的方法。代码里已经做了提前释放，注意未来修改时别破坏这个结构。

batch 回调的 tokio::spawn：如果回调 spawn 的 task 比截图周期慢很多，会累积。实测 pipeline 一次约 10-30 秒，截图周期 15 秒 × 5 = 75 秒，不会累积。但如果用户改了 interval_secs=5, batch_size=2（10 秒一批），并发处理多批就可能出问题。MVP 不做 backpressure，写到坑点里让你心里有数。

index 文件损坏：如果用户在 app 运行期间手动删了 sessions.json，下次启动会被当成全新库。解决方案是在 shutdown 里写一次，但 app 崩溃时救不了。可接受，不处理。

session_id 冲突：秒级时间戳 + 4 位 hex 后缀，理论上同一秒创建 65536 种可能，实际用户一秒内最多点一次，碰撞概率为 0。不用担心。

删除 active session 的风险：delete_session 如果删了正在运行的 session，CaptureLoop 的下一次 save_screenshot 会失败（session 在 index 里找不到了）。MVP 加一个前端守卫：删除前检查 getCaptureStatus，如果正在运行这个 session 则禁止删除。

screenshot_count 和实际文件数不一致：如果崩溃前 count 已经 +1 但文件没写成功，会偏多 1。重启后 list_screenshots 扫目录才是真相。不影响业务。

JPEG 编码 CPU 开销：4K 显示器一次编码约 100ms，频率 15 秒一次可以接受。如果以后改 5 秒一次会明显吃 CPU，到时候考虑 spawn_blocking。

## 十八、产出物

完成 spec-06 后你应该有：

截图采集完整链路：定时截图 → 存盘 → 触发 pipeline → 总结 → 写记忆 → 搜相关 → 推送
session 概念落地，可开始、可停止、可回看
一个完整可用的连接页 UI（开关 + 历史会话 + 详情页）
本地存储统计能力，为 spec-11 的存储管理做铺垫
文件系统存储的 MVP 级别完备实现，后续 P1 再引入 SQLite 时职责边界清晰
