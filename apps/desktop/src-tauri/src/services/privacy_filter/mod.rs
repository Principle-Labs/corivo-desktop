//! Privacy filter — OpenAI privacy-filter q4f16 ONNX 本地 PII 检测。
//!
//! 入口看 docs/privacy-filter-spec.md：
//! - §3 架构：classify once at capture, enforce at egress
//! - §6 component design
//! - §7 hook points (snapshot_consumer + exec_agent)
//!
//! **本模块当前是骨架**。Cargo 依赖只引入了 `blake3` + `lru`
//! (cache 层依赖)；`ort` + `tokenizers` 在推理实装那一轮加入。所以
//! [`PrivacyFilter::classify_and_redact_secrets`] 与
//! [`PrivacyFilter::enforce`] 目前都是直通 stub —— 返回原文 + 空
//! spans，调用方语义不变，build 保持绿。
//!
//! 这一层一旦实装：
//! 1. `session.rs` 加载 q4f16.onnx + tokenizer.json，懒加载 + idle
//!    unload (spec §10)。
//! 2. `decode.rs` 走 BIOES + 约束 Viterbi 把 33-class logits 转 spans。
//! 3. `cache.rs` 当前已实现：blake3 hash → spans LRU；预期 hit
//!    rate > 85% (spec §9.1)。
//! 4. `redact.rs` 拿 spans + 用户类目开关，做 char-offset 替换。
//! 5. `download.rs` 首启从 HF 拉 q4f16 + tokenizer，校验 sha256，
//!    存到 `app.path().app_data_dir()?` 下。

pub mod cache;
pub mod decode;
pub mod download;
pub mod redact;
pub mod settings;

use std::sync::Arc;

use tokio::sync::Mutex;

use crate::{
    domain::privacy::{CategoryToggles, PiiSpan, PrivacySettings},
    error::Result,
};

use cache::SpanCache;

/// Privacy filter 服务句柄。`Arc<PrivacyFilter>` 由 `lib.rs::run()`
/// 在 setup 阶段构造一次,塞进 `AppState`,后续所有 hook (`snapshot_consumer`
/// / `exec_agent`) 共享同一个 Arc —— 模型权重和缓存只占一份内存。
pub struct PrivacyFilter {
    /// 用户类目开关 + 总启用状态。Settings UI 通过 Tauri command 改
    /// 这个 (下一阶段加 commands)；hook 路径每次读 settings.is_enabled()
    /// 决定要不要短路。
    settings: Arc<Mutex<PrivacySettings>>,

    /// hash(text) → Vec<PiiSpan> LRU。spec §9.1 容量 10k 条 ≈ 5MB。
    /// 缓存逻辑独立于推理后端,即便骨架阶段也能用 (现在 classify
    /// 是 stub,缓存里只会塞空数组,不会膨胀)。
    cache: Arc<SpanCache>,
    // session: Option<Arc<RwLock<Option<ort::Session>>>>,  // 下一阶段
    // tokenizer: OnceCell<tokenizers::Tokenizer>,           // 下一阶段
}

impl PrivacyFilter {
    /// 标准构造器:用一份初始 settings 启动。settings.secret 会被强制
    /// 设为 true(spec §12.1 不可关)。
    pub fn new(mut settings: PrivacySettings) -> Self {
        settings.categories.secret = true;
        Self {
            settings: Arc::new(Mutex::new(settings)),
            cache: Arc::new(SpanCache::new()),
        }
    }

    /// 用默认(关闭)settings 构造的便捷形态。等价于
    /// `PrivacyFilter::new(PrivacySettings::default())` —— 主要给单元
    /// 测试 + 模型还没准备好时的占位用。
    pub fn new_disabled() -> Self {
        Self::new(PrivacySettings::default())
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

    /// **骨架阶段直通**:返回原文 + 空 spans。
    ///
    /// 真实语义 (下一阶段实装):
    /// 1. 走 `cache` 看 hash 命中没;
    /// 2. miss 则跑 session.run,经 decode 得到 spans;
    /// 3. 对 spans 中的 `secret` 类目,在 text 上做物理替换并把
    ///    对应 span 的 `redacted_in_storage` 标 true;
    /// 4. 写回缓存,返回 (可能改过的 text, spans)。
    ///
    /// snapshot_consumer 当前还没接这个 hook —— 接入在下一轮。
    pub async fn classify_and_redact_secrets(&self, text: &str) -> Result<(String, Vec<PiiSpan>)> {
        // 骨架:不调模型,只走缓存路径,确保 cache 模块的代码路径在
        // 单元测试 + 集成 build 中真实地被走到一次。
        let key = self.cache.key_for(text);
        if let Some(cached) = self.cache.get(&key) {
            return Ok((text.to_string(), cached));
        }
        let spans: Vec<PiiSpan> = Vec::new();
        self.cache.put(key, spans.clone());
        Ok((text.to_string(), spans))
    }

    /// Egress redact —— 拿当前 settings 快照,按用户启用类目把 spans
    /// 替换成占位符。这是异步壳;纯逻辑在 [`redact::redact`] 里,可独立
    /// 单元测试。
    ///
    /// 总开关 `enabled = false` 时短路 —— 直接返回原文,不取锁不查
    /// 表。Quick Ask / exec_agent 的 hot path 进来时,大概率走这条
    /// fast-path。
    pub async fn enforce(&self, text: &str, spans: &[PiiSpan]) -> String {
        let snap = self.settings.lock().await;
        if !snap.enabled {
            return text.to_string();
        }
        let toggles = snap.categories.clone();
        drop(snap);
        redact::redact(text, spans, &toggles)
    }

    /// 同步版本 —— caller 自己持有 toggles 快照时用。给 Phase 2
    /// "批量 redact 一组 frame" 这种场景准备的:取锁一次,跑 N 次。
    pub fn enforce_with_toggles(
        &self,
        text: &str,
        spans: &[PiiSpan],
        toggles: &CategoryToggles,
    ) -> String {
        redact::redact(text, spans, toggles)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn stub_classify_returns_input_text_unchanged() {
        let pf = PrivacyFilter::new_disabled();
        let (out, spans) = pf.classify_and_redact_secrets("hello world").await.unwrap();
        assert_eq!(out, "hello world");
        assert!(spans.is_empty());
    }

    #[tokio::test]
    async fn cache_round_trips_via_classify_stub() {
        // 第二次 classify 应该走 cache hit 路径 —— 通过 cache.len()
        // 验证只插了一次。
        let pf = PrivacyFilter::new_disabled();
        let _ = pf.classify_and_redact_secrets("一段文字").await.unwrap();
        let _ = pf.classify_and_redact_secrets("一段文字").await.unwrap();
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

    #[tokio::test]
    async fn enforce_with_disabled_settings_returns_input_unchanged() {
        // 默认 settings.enabled = false —— hot-path 短路,不应替换。
        let pf = PrivacyFilter::new_disabled();
        let spans = vec![PiiSpan {
            start: 0,
            end: 4,
            label: crate::domain::privacy::PiiLabel::PrivatePerson,
            score: 0.9,
            redacted_in_storage: false,
        }];
        let out = pf.enforce("John lives here", &spans).await;
        assert_eq!(out, "John lives here");
    }

    #[tokio::test]
    async fn enforce_with_enabled_settings_redacts() {
        let pf = PrivacyFilter::new_disabled();
        let mut s = PrivacySettings::default();
        s.enabled = true;
        pf.update_settings(s).await;
        let spans = vec![PiiSpan {
            start: 0,
            end: 4,
            label: crate::domain::privacy::PiiLabel::PrivatePerson,
            score: 0.9,
            redacted_in_storage: false,
        }];
        let out = pf.enforce("John lives here", &spans).await;
        assert_eq!(out, "[人名] lives here");
    }
}
