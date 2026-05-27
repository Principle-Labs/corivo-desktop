//! Skill 市场 Tauri 命令。
//!
//! 这一层负责：
//!   - 从 `${api_base}/v1/market/skills` 拉可见 skill 列表 (按身份过滤
//!     由 API 决定 —— 桌面端把 corivo session token 透传过去)
//!   - 把单个 skill 的 tar.gz 拉下来，**校验 sha256**，解压到
//!     `~/.corivo/skills/market/<slug>/`，并写一个 `.market-meta.json`
//!     记录源 commit SHA
//!   - 卸载 (`rm -rf` 该 slug 目录)
//!   - 列举本地已安装的 skill (读 `.market-meta.json`，用于 UI "已安装 / 可更新" 判定)
//!
//! 已安装的 skill 通过 `services::skill_share` 的标准 scan 流程被识别为
//! `SkillSource::Market`，启用后走现有 symlink sync 进
//! `$APPDATA/claude-config/skills/`，对 agent 透明。
//!
//! 安全：
//!   - sha256 钉死 (API 在 `X-Content-SHA256` header 发，本地实算比对)
//!   - tar 0.4+ `Archive::unpack` 默认拒绝 `..` 越界和绝对路径
//!   - slug 严格白名单 (lowercase + digit + hyphen)

use std::fs;
use std::io::Cursor;
use std::path::PathBuf;

use chrono::Local;
use flate2::read::GzDecoder;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tar::Archive;
use tauri::State;
use ts_rs::TS;

use crate::commands::config::AppState;
use crate::domain::ipc_error::TauriError;
use crate::env::api_base;

const MARKET_SUBPATH: &str = ".corivo/skills/market";
const META_FILENAME: &str = ".market-meta.json";
const MAX_SLUG_LEN: usize = 64;
// API 响应的 tarball 上限 —— 避免任何意外的超大 body。10 MB 远大于
// 真实 skill (几十 KB ~ 几 MB)。
const MAX_TARBALL_BYTES: u64 = 10 * 1024 * 1024;

/// 与 API `/v1/market/skills` 返回的元素 schema 对齐。
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../../packages/shared-types/src/generated/")]
#[serde(rename_all = "snake_case")]
pub struct MarketSkill {
    pub slug: String,
    pub name: String,
    pub description: String,
    pub tags: Vec<String>,
    pub kind: String,
    pub has_scripts: bool,
    pub visibility: String,
    pub commit_sha: String,
}

/// 本地 `.market-meta.json` 的 schema。也是 UI "已安装 / 可更新" 比对的
/// 信息源 —— `commit_sha` 与服务端返回的最新 `commit_sha` 不等就提示更新。
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../../packages/shared-types/src/generated/")]
#[serde(rename_all = "snake_case")]
pub struct MarketMeta {
    pub slug: String,
    pub commit_sha: String,
    pub sha256: String,
    pub installed_at: String,
}

#[derive(Debug, Deserialize)]
struct ListResp {
    skills: Vec<MarketSkill>,
}

fn market_dir() -> Result<PathBuf, TauriError> {
    let home = std::env::var_os("HOME").ok_or_else(|| TauriError::Unknown {
        message: "HOME env var is not set; skill market install requires it".to_string(),
    })?;
    Ok(PathBuf::from(home).join(MARKET_SUBPATH))
}

fn validate_slug(slug: &str) -> Result<(), TauriError> {
    if slug.is_empty() || slug.len() > MAX_SLUG_LEN {
        return Err(TauriError::Unknown {
            message: format!("invalid slug length: {}", slug.len()),
        });
    }
    let bytes = slug.as_bytes();
    if bytes[0] == b'-' || *bytes.last().unwrap() == b'-' {
        return Err(TauriError::Unknown {
            message: "slug must not start/end with hyphen".to_string(),
        });
    }
    if !slug
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    {
        return Err(TauriError::Unknown {
            message: format!("slug contains invalid char: {slug}"),
        });
    }
    Ok(())
}

fn http_client() -> Client {
    Client::builder()
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .expect("reqwest Client::build infallible with defaults")
}

/// Pull the live corivo session access token off Config. None when the
/// user is signed out — calls then go through anonymous and the API
/// returns only public skills.
fn session_token(state: &AppState) -> Option<String> {
    state
        .config_service
        .get()
        .corivo_session
        .access_token
        .filter(|t| !t.is_empty())
}

fn apply_auth(req: reqwest::RequestBuilder, token: Option<&str>) -> reqwest::RequestBuilder {
    match token {
        Some(t) if !t.is_empty() => req.bearer_auth(t),
        _ => req,
    }
}

#[tauri::command]
pub async fn skill_market_list(
    state: State<'_, AppState>,
) -> Result<Vec<MarketSkill>, TauriError> {
    let token = session_token(&state);
    let url = format!("{}/v1/market/skills", api_base());
    let req = apply_auth(http_client().get(&url), token.as_deref());
    let resp = req.send().await.map_err(|e| TauriError::Unknown {
        message: format!("market.list request failed: {e}"),
    })?;
    if !resp.status().is_success() {
        return Err(TauriError::Unknown {
            message: format!("market.list returned {}", resp.status()),
        });
    }
    let parsed: ListResp = resp.json().await.map_err(|e| TauriError::Unknown {
        message: format!("market.list parse failed: {e}"),
    })?;
    Ok(parsed.skills)
}

#[tauri::command]
pub async fn skill_market_install(
    state: State<'_, AppState>,
    slug: String,
) -> Result<MarketMeta, TauriError> {
    validate_slug(&slug)?;
    let token = session_token(&state);
    let url = format!("{}/v1/market/skills/{}/download", api_base(), slug);
    let req = apply_auth(http_client().get(&url), token.as_deref());
    let resp = req.send().await.map_err(|e| TauriError::Unknown {
        message: format!("market.install request failed: {e}"),
    })?;
    if !resp.status().is_success() {
        return Err(TauriError::Unknown {
            message: format!("market.install returned {}", resp.status()),
        });
    }

    // 防御超大 body —— 服务端不该发，但客户端要自保。
    if let Some(len) = resp.content_length() {
        if len > MAX_TARBALL_BYTES {
            return Err(TauriError::Unknown {
                message: format!("tarball too large: {len} bytes"),
            });
        }
    }

    let commit_sha = header_str(&resp, "X-Commit-SHA")?;
    let expected_sha = header_str(&resp, "X-Content-SHA256")?;

    let bytes = resp.bytes().await.map_err(|e| TauriError::Unknown {
        message: format!("market.install body read failed: {e}"),
    })?;
    if bytes.len() as u64 > MAX_TARBALL_BYTES {
        return Err(TauriError::Unknown {
            message: format!("tarball too large: {} bytes", bytes.len()),
        });
    }

    // sha256 钉死。任何不一致都中断 —— 不写盘、不留半成品。
    let actual_sha = format!("{:x}", Sha256::digest(&bytes));
    if actual_sha != expected_sha {
        return Err(TauriError::Unknown {
            message: format!(
                "sha256 mismatch: expected {expected_sha}, got {actual_sha}"
            ),
        });
    }

    // 落盘：先解到临时同级目录、再原子 rename，避免半成品污染主目录。
    let dest = market_dir()?.join(&slug);
    let parent = dest
        .parent()
        .ok_or_else(|| TauriError::Unknown {
            message: "market dir has no parent".to_string(),
        })?
        .to_path_buf();
    fs::create_dir_all(&parent).map_err(|e| TauriError::Unknown {
        message: format!("market.install mkdir failed: {e}"),
    })?;

    let tmp = parent.join(format!(".{slug}.tmp"));
    if tmp.exists() {
        fs::remove_dir_all(&tmp).map_err(|e| TauriError::Unknown {
            message: format!("market.install cleanup tmp failed: {e}"),
        })?;
    }
    fs::create_dir_all(&tmp).map_err(|e| TauriError::Unknown {
        message: format!("market.install mkdir tmp failed: {e}"),
    })?;

    let gz = GzDecoder::new(Cursor::new(bytes.as_ref()));
    let mut archive = Archive::new(gz);
    archive.set_preserve_permissions(false);
    archive.set_overwrite(true);
    if let Err(err) = archive.unpack(&tmp) {
        let _ = fs::remove_dir_all(&tmp);
        return Err(TauriError::Unknown {
            message: format!("market.install unpack failed: {err}"),
        });
    }

    // 老安装直接覆盖。先 rename 旧 dir → 旧.bak，新 tmp → dest，最后清掉 .bak。
    let bak = parent.join(format!(".{slug}.bak"));
    if dest.exists() {
        let _ = fs::remove_dir_all(&bak);
        fs::rename(&dest, &bak).map_err(|e| TauriError::Unknown {
            message: format!("market.install rename old dir failed: {e}"),
        })?;
    }
    if let Err(err) = fs::rename(&tmp, &dest) {
        // 回滚
        if bak.exists() {
            let _ = fs::rename(&bak, &dest);
        }
        return Err(TauriError::Unknown {
            message: format!("market.install rename tmp failed: {err}"),
        });
    }
    let _ = fs::remove_dir_all(&bak);

    let meta = MarketMeta {
        slug: slug.clone(),
        commit_sha,
        sha256: actual_sha,
        installed_at: Local::now().to_rfc3339(),
    };
    let meta_path = dest.join(META_FILENAME);
    fs::write(
        &meta_path,
        serde_json::to_string_pretty(&meta).expect("meta serialization infallible"),
    )
    .map_err(|e| TauriError::Unknown {
        message: format!("market.install write meta failed: {e}"),
    })?;

    tracing::info!(slug = %slug, commit_sha = %meta.commit_sha, "market.installed");
    Ok(meta)
}

#[tauri::command]
pub async fn skill_market_uninstall(slug: String) -> Result<(), TauriError> {
    validate_slug(&slug)?;
    let dir = market_dir()?.join(&slug);
    if dir.exists() {
        fs::remove_dir_all(&dir).map_err(|e| TauriError::Unknown {
            message: format!("market.uninstall rm failed: {e}"),
        })?;
        tracing::info!(slug = %slug, "market.uninstalled");
    }
    Ok(())
}

#[tauri::command]
pub async fn skill_market_installed() -> Result<Vec<MarketMeta>, TauriError> {
    let dir = match market_dir() {
        Ok(d) => d,
        // HOME 未设：等价于"什么都没装"。
        Err(_) => return Ok(Vec::new()),
    };
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    let entries = match fs::read_dir(&dir) {
        Ok(e) => e,
        Err(_) => return Ok(Vec::new()),
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        // 跳过临时目录 (`.tmp`, `.bak`) 和 dotfiles。
        if name_str.starts_with('.') {
            continue;
        }
        let meta_path = path.join(META_FILENAME);
        let Ok(text) = fs::read_to_string(&meta_path) else {
            continue;
        };
        if let Ok(meta) = serde_json::from_str::<MarketMeta>(&text) {
            out.push(meta);
        }
    }
    out.sort_by(|a, b| a.slug.cmp(&b.slug));
    Ok(out)
}

fn header_str(resp: &reqwest::Response, name: &str) -> Result<String, TauriError> {
    resp.headers()
        .get(name)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string())
        .ok_or_else(|| TauriError::Unknown {
            message: format!("market.install missing header: {name}"),
        })
}
