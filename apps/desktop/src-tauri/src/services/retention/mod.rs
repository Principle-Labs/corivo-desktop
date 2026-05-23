//! Background retention task (spec §五 + §九 「数据保留」).
//!
//! Drops frames + screenshots + agent-session jsonl files older than
//! `RETENTION_DAYS` so the local footprint stays bounded. Tuned gentle
//! on purpose:
//!
//! - Tick interval: `RETENTION_INTERVAL` (1 hour).
//! - DB pass: `FrameRepo::delete_older_than` cascades into
//!   `frame_embeddings` via FK.
//! - Filesystem pass: `CaptureStore::delete_older_than(RETENTION_DAYS)`
//!   removes whole inactive sessions whose `started_at` is past the
//!   cutoff.
//! - JSONL pass (spec §8.1): drops `corivo-agent-sessions/*.jsonl`
//!   whose mtime is past the cutoff. Aligned with frame retention —
//!   once the frames a session cited are gone, the session can't be
//!   meaningfully resumed anyway.
//!
//! Failures are logged and swallowed — capture is the priority, retention
//! is eventually consistent. The first round runs after the first sleep
//! so we don't hammer the DB during boot.

use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::{Duration, SystemTime};

use chrono::Utc;
use tokio::{sync::Mutex, task::JoinHandle, time::sleep};

use crate::{db::repos::frames::FrameRepo, error::Result, services::capture_store::CaptureStore};

/// 90 天 retention 窗口 (spec §五 + §九). 用户可以调，但 Phase 5 启动
/// 时硬编码 — Settings UI 里只暴露 "立即清空"。
pub const RETENTION_DAYS: u32 = 90;
const RETENTION_INTERVAL: Duration = Duration::from_secs(3600);

pub struct RetentionTask {
    frames: Arc<dyn FrameRepo>,
    capture_store: Arc<CaptureStore>,
    /// Spec §8.1 root for pi-coding-agent jsonl session files. Owned
    /// by Rust because boot resolves it once via
    /// `app.path().app_data_dir()`. `None` when AppData resolution
    /// failed at boot — retention still sweeps frames/screenshots.
    sessions_dir: Option<PathBuf>,
    running: Arc<AtomicBool>,
    worker: Mutex<Option<JoinHandle<()>>>,
}

impl RetentionTask {
    pub fn new(
        frames: Arc<dyn FrameRepo>,
        capture_store: Arc<CaptureStore>,
        sessions_dir: Option<PathBuf>,
    ) -> Self {
        Self {
            frames,
            capture_store,
            sessions_dir,
            running: Arc::new(AtomicBool::new(false)),
            worker: Mutex::new(None),
        }
    }

    pub async fn start(&self) {
        if self.running.swap(true, Ordering::SeqCst) {
            return;
        }
        let frames = self.frames.clone();
        let store = self.capture_store.clone();
        let sessions_dir = self.sessions_dir.clone();
        let running = self.running.clone();
        let task = tokio::spawn(async move {
            run_loop(frames, store, sessions_dir, running).await;
        });
        *self.worker.lock().await = Some(task);
    }

    pub async fn stop(&self) {
        self.running.store(false, Ordering::SeqCst);
        if let Some(task) = self.worker.lock().await.take() {
            task.abort();
        }
    }
}

async fn run_loop(
    frames: Arc<dyn FrameRepo>,
    capture_store: Arc<CaptureStore>,
    sessions_dir: Option<PathBuf>,
    running: Arc<AtomicBool>,
) {
    while running.load(Ordering::SeqCst) {
        // Sleep first so retention doesn't fire during boot — the user
        // just opened the app and we'd rather they not see disk thrash.
        sleep(RETENTION_INTERVAL).await;
        if !running.load(Ordering::SeqCst) {
            break;
        }
        if let Err(error) = run_round(&*frames, &capture_store, sessions_dir.as_deref()).await {
            tracing::warn!(?error, "retention.round_failed");
        }
    }
    tracing::info!("retention.loop_exit");
}

/// One retention pass — DB drop, filesystem drop, then jsonl drop.
/// DB first so an interrupted round leaves orphan files (cleaned up
/// on the next round) rather than orphan rows pointing at missing
/// screenshots.
pub async fn run_round(
    frames: &dyn FrameRepo,
    capture_store: &CaptureStore,
    sessions_dir: Option<&Path>,
) -> Result<()> {
    let cutoff = Utc::now() - chrono::Duration::days(RETENTION_DAYS as i64);
    let removed_frames = frames.delete_older_than(cutoff).await?;
    let removed_sessions = capture_store.delete_older_than(RETENTION_DAYS).await?;
    let removed_jsonl = match sessions_dir {
        Some(dir) => {
            let cutoff_st = SystemTime::now()
                .checked_sub(Duration::from_secs(86_400 * RETENTION_DAYS as u64))
                .unwrap_or(SystemTime::UNIX_EPOCH);
            sweep_agent_sessions_dir(dir, cutoff_st).await
        }
        None => 0,
    };
    if removed_frames > 0 || removed_sessions > 0 || removed_jsonl > 0 {
        tracing::info!(
            removed_frames,
            removed_sessions,
            removed_jsonl,
            cutoff = %cutoff,
            "retention.round_complete"
        );
    }
    Ok(())
}

/// Drop `*.jsonl` files in `dir` whose mtime is older than `cutoff`.
/// Off the tokio reactor — directory scans are synchronous and short.
async fn sweep_agent_sessions_dir(dir: &Path, cutoff: SystemTime) -> u32 {
    let dir = dir.to_path_buf();
    tokio::task::spawn_blocking(move || sweep_agent_sessions_dir_blocking(&dir, cutoff))
        .await
        .unwrap_or_else(|e| {
            tracing::warn!(?e, "retention.jsonl_sweep_join_failed");
            0
        })
}

fn sweep_agent_sessions_dir_blocking(dir: &Path, cutoff: SystemTime) -> u32 {
    if !dir.exists() {
        // Spec §8.1 dir is created lazily by the runner on first turn —
        // a brand-new install hasn't produced one yet.
        return 0;
    }
    let entries = match std::fs::read_dir(dir) {
        Ok(it) => it,
        Err(e) => {
            tracing::warn!(?e, path = %dir.display(), "retention.jsonl_readdir_failed");
            return 0;
        }
    };
    let mut removed: u32 = 0;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("jsonl") {
            continue;
        }
        let metadata = match entry.metadata() {
            Ok(m) => m,
            Err(_) => continue,
        };
        if !metadata.is_file() {
            continue;
        }
        let mtime = match metadata.modified() {
            Ok(t) => t,
            Err(_) => continue,
        };
        if mtime > cutoff {
            continue;
        }
        if let Err(e) = std::fs::remove_file(&path) {
            tracing::warn!(?e, path = %path.display(), "retention.jsonl_remove_failed");
            continue;
        }
        removed += 1;
    }
    removed
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::{self, File};
    use std::io::Write;

    fn write_jsonl(dir: &Path, name: &str) -> PathBuf {
        let p = dir.join(name);
        let mut f = File::create(&p).unwrap();
        f.write_all(b"{}\n").unwrap();
        p
    }

    #[test]
    fn sweep_keeps_recent_jsonl_when_cutoff_is_in_the_past() {
        // With cutoff = epoch, no file's mtime is ever older than the
        // cutoff (any real file's mtime is > 1970), so nothing gets
        // removed. Verifies the comparator and extension filter without
        // needing an mtime-mutation crate.
        let dir = tempfile::tempdir().unwrap();
        let a = write_jsonl(dir.path(), "a.jsonl");
        let b = write_jsonl(dir.path(), "b.jsonl");
        let unrelated = dir.path().join("notes.txt");
        File::create(&unrelated).unwrap();

        let removed = sweep_agent_sessions_dir_blocking(dir.path(), SystemTime::UNIX_EPOCH);
        assert_eq!(removed, 0);
        assert!(a.exists());
        assert!(b.exists());
        assert!(unrelated.exists());
    }

    #[test]
    fn sweep_drops_jsonl_when_cutoff_is_far_future_and_skips_other_extensions() {
        // With cutoff in the future, any file's mtime <= cutoff, so all
        // *.jsonl files get removed. Files with other extensions are
        // left alone — defensive against accidentally clobbering
        // sibling state someone might drop in this dir.
        let dir = tempfile::tempdir().unwrap();
        let a = write_jsonl(dir.path(), "thread-1.jsonl");
        let b = write_jsonl(dir.path(), "thread-2.jsonl");
        let unrelated = dir.path().join("notes.txt");
        File::create(&unrelated).unwrap();

        let future = SystemTime::now() + Duration::from_secs(60 * 60 * 24 * 365);
        let removed = sweep_agent_sessions_dir_blocking(dir.path(), future);
        assert_eq!(removed, 2);
        assert!(!a.exists());
        assert!(!b.exists());
        assert!(unrelated.exists());
    }

    #[test]
    fn sweep_handles_missing_dir() {
        let nonexistent = std::env::temp_dir().join("corivo-retention-not-here");
        // Defensive: even if it exists, ensure it doesn't.
        let _ = fs::remove_dir_all(&nonexistent);
        let removed = sweep_agent_sessions_dir_blocking(&nonexistent, SystemTime::now());
        assert_eq!(removed, 0);
    }
}
