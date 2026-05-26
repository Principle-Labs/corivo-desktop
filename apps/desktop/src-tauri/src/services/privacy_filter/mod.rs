//! Privacy filter —— OpenAI privacy-filter q4f16 ONNX 本地 PII 检测。
//!
//! 入口看 docs/privacy-filter-spec.md。**v1600 架构调整**:撤掉
//! classify-at-capture (Hook A)。现在只剩一刀 —— `exec_agent.rs` 出口
//! 处调 [`PrivacyFilter::classify_and_enforce`],由本模块负责
//! cache lookup → (miss 跑模型) → 用当前 settings toggles 做 redact。
//!
//! PII spans 不再落盘任何表;LRU 缓存命中放大重复 prompt(同一 thread 内
//! `<persistent_memory>` / focus_context.primary_text 反复 egress)的吞吐。
//!
//! ## 实装组件(全部就位)
//!
//! 1. [`session`] - q4f16 ONNX + tokenizer + config.json 懒加载,
//!    第一次 classify 时把 sentence transformer 装进内存(spec §10),
//!    后续 classify 走 read lock + 串行 inference。模型不在 / corrupt
//!    时所有 classify 都安全降级成空 spans + warning。
//! 2. [`decode`] - 33-class BIOES greedy decode → token spans。
//! 3. [`cache`] - blake3(text) → spans LRU(spec §9.1 容量 10k 条)。
//! 4. [`redact`] - char-offset replace with user toggles。
//! 5. [`download`] - HF 拉模型 + sha256 校验,commands/privacy.rs
//!    暴露给前端。

pub mod cache;
pub mod decode;
pub mod download;
pub mod redact;
pub mod session;
pub mod settings;

use std::path::PathBuf;
use std::sync::Arc;

use tokio::sync::Mutex;

use crate::domain::privacy::{PiiLabel, PrivacySettings};

use cache::SpanCache;
use session::PrivacySession;

/// Privacy filter 服务句柄。`Arc<PrivacyFilter>` 由 `lib.rs::run()`
/// 在 setup 阶段构造一次,塞进 `AppState`,后续所有 hook (`exec_agent`)
/// 共享同一个 Arc —— 模型权重和缓存只占一份内存。
pub struct PrivacyFilter {
    /// 用户类目开关 + 总启用状态。Settings UI 通过 Tauri command 改
    /// 这个;hook 路径每次读 settings.enabled 决定要不要短路。
    settings: Arc<Mutex<PrivacySettings>>,

    /// hash(text) → Vec<PiiSpan> LRU。spec §9.1 容量 10k 条 ≈ 5MB。
    cache: Arc<SpanCache>,

    /// ONNX session 容器。首次 classify_and_enforce 触发懒加载。
    /// 模型文件不在 / corrupt 时 session.classify 返回空 vec —— hot
    /// path 自然降级成"过原文出去"。
    session: Arc<PrivacySession>,
}

impl PrivacyFilter {
    /// 标准构造器。`model_dir` 是 `download::model_dir(app_data_dir,
    /// MODEL_MANIFEST)` 给出的路径 —— 即
    /// `$APP_DATA/models/privacy-filter-q4f16/`。settings.secret 会被
    /// 强制设为 true(spec §12.1 不可关)。
    pub fn new(mut settings: PrivacySettings, model_dir: PathBuf) -> Self {
        settings.categories.secret = true;
        Self {
            settings: Arc::new(Mutex::new(settings)),
            cache: Arc::new(SpanCache::new()),
            session: Arc::new(PrivacySession::new(model_dir)),
        }
    }

    /// 用默认(关闭)settings + 不存在的 model_dir 构造的便捷形态。
    /// 主要给单元测试用 —— session.classify 在 model 文件缺失时返回
    /// 空 vec,不会 panic,所以测试可以放心调任何方法。
    pub fn new_disabled() -> Self {
        Self::new(
            PrivacySettings::default(),
            PathBuf::from("__privacy_filter_test_no_model__"),
        )
    }

    /// 当前生效的用户偏好快照。
    pub async fn settings_snapshot(&self) -> PrivacySettings {
        self.settings.lock().await.clone()
    }

    /// 覆盖用户偏好 —— Settings UI 调这个。`secret` 字段在写入时
    /// 强制成 true (spec §12.1 不可关)。
    pub async fn update_settings(&self, mut new: PrivacySettings) {
        new.categories.secret = true;
        *self.settings.lock().await = new;
    }

    /// 清空缓存 —— Settings 页"清除隐私缓存"按钮调这个,以及
    /// 模型升级后避免旧 spans 跟新模型不一致。
    pub fn clear_cache(&self) {
        self.cache.clear();
    }

    /// 同时卸载已加载的 ONNX session —— delete_privacy_model 完整
    /// 路径用,清干净 RSS。
    pub async fn unload_session(&self) {
        self.session.unload().await;
    }

    /// Egress classify + redact —— exec_agent.rs 出口处调这个。
    ///
    /// 流程:
    /// 1. settings.enabled=false → fast-path 返回原文,不取 cache 锁也
    ///    不跑模型。
    /// 2. cache lookup (blake3(text) → spans)。
    /// 3. miss:`session.classify(text)` 跑 ONNX → cache.put → 用 spans。
    ///    模型未下载 / 加载失败时 session 返回空 vec,redact 是 no-op,
    ///    文本直通返回。
    /// 4. 用当前 settings.categories 做 redact,返回替换后的字符串。
    ///
    /// `secret` 类目永远 on (spec §12.1),即便用户在 UI 上手动关掉
    /// `categories.secret`,这里读到的也是 true (update_settings 强制)。
    pub async fn classify_and_enforce(&self, text: &str) -> String {
        let snap = self.settings.lock().await;
        if !snap.enabled {
            return text.to_string();
        }
        let toggles = snap.categories.clone();
        drop(snap);

        let key = self.cache.key_for(text);
        let (spans, cache_hit) = if let Some(cached) = self.cache.get(&key) {
            (cached, true)
        } else {
            let spans = self.session.classify(text).await;
            self.cache.put(key, spans.clone());
            (spans, false)
        };

        let out = redact::redact(text, &spans, &toggles);

        // Per-call diagnostic. **Never** log original text, redacted output,
        // or span text slices — that would leak the very PII the model just
        // identified back into the log. Only non-content signals: input
        // length, cache hit, span count, label histogram, and whether the
        // output differs. The enabled!() guard skips HashMap allocation
        // when no DEBUG subscriber is attached.
        if tracing::enabled!(target: "privacy_filter", tracing::Level::DEBUG) {
            let mut labels: std::collections::HashMap<PiiLabel, usize> =
                std::collections::HashMap::new();
            for s in &spans {
                *labels.entry(s.label).or_insert(0) += 1;
            }
            tracing::debug!(
                target: "privacy_filter",
                text_len = text.len(),
                cache_hit,
                count = spans.len(),
                labels = ?labels,
                changed = out != text,
                "enforce.spans"
            );
        }

        // Content-exposing diagnostic — gated at TRACE so it never fires by
        // default. The very text on these two lines is the PII the model
        // just identified, so anything that ingests the log file
        // (file-layer scrape, Sentry breadcrumb, shoulder-surfing) sees
        // raw user data. Enable explicitly with e.g.
        //   $env:RUST_LOG = "privacy_filter=trace,debug"
        // before launching `pnpm app:dev`. File layer is INFO+ so even
        // when TRACE is on this still won't land on disk.
        if tracing::enabled!(target: "privacy_filter", tracing::Level::TRACE) {
            tracing::trace!(
                target: "privacy_filter",
                input = %text,
                output = %out,
                "enforce.content"
            );
        }

        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn disabled_settings_returns_input_unchanged() {
        // 默认 settings.enabled = false —— hot-path 短路,不取 cache
        // 锁,不跑模型,直接返回。
        let pf = PrivacyFilter::new_disabled();
        let out = pf.classify_and_enforce("John lives here").await;
        assert_eq!(out, "John lives here");
        // 短路时也不应该往 cache 写
        assert!(pf.cache.is_empty());
    }

    #[tokio::test]
    async fn enabled_without_model_returns_input_unchanged() {
        // new_disabled 用一个不存在的 model_dir,session.classify 在
        // 文件缺失时返回空 vec,redact 是 no-op —— enable 之后仍然返回
        // 原文。线上模型就位后这条测试要换成 expect 替换后的字符串。
        let pf = PrivacyFilter::new_disabled();
        let mut s = PrivacySettings::default();
        s.enabled = true;
        pf.update_settings(s).await;

        let out = pf.classify_and_enforce("John lives here").await;
        assert_eq!(out, "John lives here");
    }

    #[tokio::test]
    async fn cache_round_trips_on_repeated_classify() {
        // 同一段文本调两次 classify_and_enforce —— 第二次应该走 cache hit
        // 路径(cache.len 不增加)。session 失败时也会走 cache.put(空 vec
        // 也是有效的"已 classify"语义),所以这条测试不依赖模型存在。
        let pf = PrivacyFilter::new_disabled();
        let mut s = PrivacySettings::default();
        s.enabled = true;
        pf.update_settings(s).await;

        let _ = pf.classify_and_enforce("一段文字").await;
        let _ = pf.classify_and_enforce("一段文字").await;
        assert_eq!(pf.cache.len(), 1);
    }

    #[tokio::test]
    async fn update_settings_forces_secret_on() {
        let pf = PrivacyFilter::new_disabled();
        let mut s = PrivacySettings::default();
        s.categories.secret = false; // 用户尝试关掉 secret
        pf.update_settings(s).await;
        assert!(pf.settings_snapshot().await.categories.secret);
    }
}
