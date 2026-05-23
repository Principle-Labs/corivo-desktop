//! Tauri commands —— Settings UI 跟 `services::privacy_filter` 的桥。
//!
//! 六个面:
//! 1. `get_privacy_settings` —— 读当前生效的偏好(从内存,不走 disk)
//! 2. `set_privacy_settings` —— 用户在 Settings 切换开关时写入。
//!    走 read-modify-write under `config_service` 的锁,持久化到
//!    `config.json`,**并**同步 `PrivacyFilter` 的内存快照,确保下一次
//!    `enforce` 立刻看到新值。
//! 3. `clear_privacy_cache` —— 用户在 Settings 点"清除隐私缓存"或者
//!    我们后续做模型升级时调用,把 blake3 → spans 的 LRU 清空。
//! 4. `get_privacy_model_status` —— Settings UI 渲染时读一次,告诉用户
//!    "模型是否已下载 / 大约多大 / 加载后占多少 RAM / 来源 URL"。
//! 5. `download_privacy_model` —— 用户在 Settings 点"启用 + 下载"时调
//!    用,通过 `Channel<ModelDownloadProgress>` 把进度实时流给 UI。
//! 6. `delete_privacy_model` —— 删除模型目录(磁盘清理 / 重新下载场景)。

use std::sync::Arc;

use tauri::ipc::Channel;
use tauri::{AppHandle, Emitter, Manager, Runtime, State};

use crate::{
    commands::config::AppState,
    domain::{
        config::ConfigChanged,
        privacy::{
            ModelDownloadProgress, PrivacyModelStatus, PrivacySettings,
            PRIVACY_MODEL_RAM_ESTIMATE_BYTES, PRIVACY_MODEL_SOURCE_URL,
        },
    },
    services::privacy_filter::download::{self, ProgressCallback, MODEL_MANIFEST},
};

/// 读当前 settings 快照。React Query 的 `queryFn` 用 —— cheap,
/// 纯内存读。
#[tauri::command]
pub async fn get_privacy_settings(state: State<'_, AppState>) -> Result<PrivacySettings, String> {
    Ok(state.privacy_filter.settings_snapshot().await)
}

/// 用户在 Settings 写入新偏好。两步:
/// 1. 持久化到 `config.json`(read-modify-write 走 config_service 的锁)
/// 2. 同步 PrivacyFilter 的内存快照,让 hot-path 立刻看到新值
///
/// `secret` 字段在两边都会被强制成 true(spec §12.1)。
#[tauri::command]
pub async fn set_privacy_settings(
    app: AppHandle,
    state: State<'_, AppState>,
    settings: PrivacySettings,
) -> Result<PrivacySettings, String> {
    // 强制 secret on —— 避免任何路径让它漏出 off 状态
    let mut new_settings = settings;
    new_settings.categories.secret = true;

    // 1. 持久化
    let mut config = state.config_service.get();
    config.privacy_filter = new_settings.clone();
    state.config_service.update(config).map_err(String::from)?;

    // 2. 同步 in-memory snapshot
    state
        .privacy_filter
        .update_settings(new_settings.clone())
        .await;

    emit_config_changed(&app);
    Ok(new_settings)
}

/// 清空 hash → spans LRU。
///
/// 适用场景:
/// - 用户主动点 Settings 里"清除隐私缓存"按钮
/// - 模型权重升级后 —— 旧缓存里的 spans 可能跟新模型解码不一致
///   (`services::privacy_filter::download` 升级路径走完后调一次)
#[tauri::command]
pub async fn clear_privacy_cache(state: State<'_, AppState>) -> Result<(), String> {
    state.privacy_filter.clear_cache();
    Ok(())
}

/// 模型整体状态快照 —— Settings UI 进入页面 + 下载完成后各读一次。
///
/// `downloaded` 走 `download::is_ready`,所以"文件存在但 sha256 校验
/// 不过"会被报成 `false`(等到 sha256 被填进 MODEL_MANIFEST 之后这条
/// 才会真正生效;在那之前等价于"文件存在 + 大小匹配")。
#[tauri::command]
pub async fn get_privacy_model_status(app: AppHandle) -> Result<PrivacyModelStatus, String> {
    let base = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("app_data_dir: {e}"))?;
    let downloaded = download::is_ready(&base, &MODEL_MANIFEST).await;
    Ok(PrivacyModelStatus {
        downloaded,
        variant: MODEL_MANIFEST.variant_dir.to_string(),
        total_size_bytes: MODEL_MANIFEST.total_size_bytes(),
        ram_estimate_bytes: PRIVACY_MODEL_RAM_ESTIMATE_BYTES,
        source_url: PRIVACY_MODEL_SOURCE_URL.to_string(),
    })
}

/// 触发首启下载。**不**自动改 `settings.enabled` —— 调用方
/// (Settings UI)在下载完成后用 `set_privacy_settings({enabled: true,
/// ...})` 显式启用。这样"开关切到 on 之前下载失败"的场景里用户的
/// 偏好状态不会被悄悄改写。
///
/// 进度通过 `progress: Channel<ModelDownloadProgress>` 流式回前端 ——
/// 每个 chunk 一条事件,目前节流为"按 chunk 触发",量大但够 UI 用。
/// 后续可以加 throttle(`Debouncer`)。
///
/// 失败时:Result::Err 返回详细消息;UI 应该 toast 用户 + 保持
/// `enabled = false`。
#[tauri::command]
pub async fn download_privacy_model(
    app: AppHandle,
    progress: Channel<ModelDownloadProgress>,
) -> Result<(), String> {
    let base = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("app_data_dir: {e}"))?;

    // 已就绪就直接返回 —— 调用方应该已经在 `get_privacy_model_status`
    // 的基础上决定要不要触发,但这里再做一道闸门更安全。
    if download::is_ready(&base, &MODEL_MANIFEST).await {
        return Ok(());
    }

    // 包装进度回调:每个文件按"已下载 / 总大小 / file_index / file_count"
    // 推给 UI。Arc 共享给后台任务。
    let file_count = MODEL_MANIFEST.file_count();
    let file_index = Arc::new(std::sync::atomic::AtomicU32::new(0));
    let last_file_name = Arc::new(std::sync::Mutex::new(String::new()));
    let progress_arc = Arc::new(progress);

    let cb: ProgressCallback = {
        let progress = progress_arc.clone();
        let file_index = file_index.clone();
        let last_file_name = last_file_name.clone();
        Arc::new(move |name: &str, downloaded: u64, total: u64| {
            // 文件名变化时,file_index 自增 —— manifest 是串行下载的,
            // 所以每碰到新文件等于切到下一项。
            let mut last = last_file_name.lock().unwrap();
            if last.as_str() != name {
                file_index.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                *last = name.to_string();
            }
            drop(last);
            let _ = progress.send(ModelDownloadProgress {
                file_name: name.to_string(),
                downloaded_bytes: downloaded,
                total_bytes: total,
                file_index: file_index.load(std::sync::atomic::Ordering::SeqCst),
                file_count,
            });
        })
    };

    download::ensure_downloaded(&base, &MODEL_MANIFEST, Some(cb))
        .await
        .map_err(|e| format!("download failed: {e}"))?;

    Ok(())
}

/// 删掉模型目录。"重新下载"+"完全清理"两个 UI 按钮都走它。
///
/// **自动同步 settings.enabled = false** —— 删模型时用户的开关也得
/// 跟着关掉,不然之后会出现"开关 on 但模型不存在"的不一致状态,后续
/// 的 PrivacyFilter::enforce 会拿到空 spans + 用户以为已经在保护。
#[tauri::command]
pub async fn delete_privacy_model(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let base = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("app_data_dir: {e}"))?;

    download::delete_model_dir(&base, &MODEL_MANIFEST)
        .await
        .map_err(|e| format!("delete model dir: {e}"))?;

    // 同步关掉总开关 —— 走 set_privacy_settings 的完整路径
    // (持久化 + in-memory snapshot + emit event),保持不变量。
    let mut new_settings = state.privacy_filter.settings_snapshot().await;
    new_settings.enabled = false;
    let mut config = state.config_service.get();
    config.privacy_filter = new_settings.clone();
    state.config_service.update(config).map_err(String::from)?;
    state.privacy_filter.update_settings(new_settings).await;

    emit_config_changed(&app);
    Ok(())
}

fn emit_config_changed<R: Runtime>(app: &AppHandle<R>) {
    if let Err(error) = app.emit(ConfigChanged::EVENT, &()) {
        tracing::warn!(?error, "privacy_filter.emit_config_changed_failed");
    }
}
